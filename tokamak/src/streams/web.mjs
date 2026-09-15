const internalToken = Symbol("stream-internal");

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  promise.catch(() => {});
  return { promise, resolve, reject };
}

function cloneChunk(value) {
  if (value instanceof ArrayBuffer) return value.slice(0);
  if (ArrayBuffer.isView(value)) {
    return new Uint8Array(value.buffer, value.byteOffset, value.byteLength).slice();
  }
  return value;
}

function rejected(error) {
  return Promise.reject(error);
}

function normalizeStrategy(strategy, defaultHighWaterMark) {
  if (strategy === null || (typeof strategy !== "object" && typeof strategy !== "function")) {
    throw new TypeError("Stream strategy must be an object");
  }
  const highWaterMark = strategy.highWaterMark === undefined
    ? defaultHighWaterMark
    : Number(strategy.highWaterMark);
  if (Number.isNaN(highWaterMark) || highWaterMark < 0) {
    throw new RangeError("Invalid highWaterMark");
  }
  const size = strategy.size === undefined ? () => 1 : strategy.size;
  if (typeof size !== "function") throw new TypeError("Stream size must be a function");
  return { highWaterMark, size };
}

function createController(stream, prototype) {
  const controller = Object.create(prototype);
  Object.defineProperty(controller, "__stream", { value: stream });
  return controller;
}

class ReadRequest {
  constructor(view, autoAllocate = false, min = 1) {
    this.view = view;
    this.autoAllocate = autoAllocate;
    this.min = min;
    this.written = 0;
    this.deferred = deferred();
  }
}

export class ReadableStream {
  constructor(source = {}, strategy = {}) {
    if (source === null || (typeof source !== "object" && typeof source !== "function")) {
      throw new TypeError("ReadableStream source must be an object");
    }
    if (source.type !== undefined && source.type !== "bytes") {
      throw new TypeError("ReadableStream type must be bytes");
    }
    this.__source = source;
    this.__type = source.type === "bytes" ? "bytes" : "default";
    if (this.__type === "bytes" && strategy?.size !== undefined) {
      throw new RangeError("Byte stream strategies cannot define size");
    }
    this.__autoAllocateChunkSize = this.__type === "bytes" ? source.autoAllocateChunkSize : undefined;
    if (this.__autoAllocateChunkSize !== undefined) {
      const size = Number(this.__autoAllocateChunkSize);
      if (!Number.isSafeInteger(size) || size <= 0) throw new RangeError("Invalid autoAllocateChunkSize");
      this.__autoAllocateChunkSize = size;
    }
    const strategyData = normalizeStrategy(strategy, this.__type === "bytes" ? 0 : 1);
    this.__strategy = this.__type === "bytes"
      ? { highWaterMark: strategyData.highWaterMark, size: value => value.byteLength }
      : strategyData;
    this.__state = "readable";
    this.__storedError = undefined;
    this.__queue = [];
    this.__queueSize = 0;
    this.__readRequests = [];
    this.__reader = null;
    this.__started = false;
    this.__pulling = false;
    this.__pullAgain = false;
    this.__disturbed = false;
    this.__cancelPromise = null;
    this.__startPromise = Promise.resolve();
    this.__controller = createController(
      this,
      this.__type === "bytes" ? ReadableByteStreamController.prototype : ReadableStreamDefaultController.prototype,
    );

    try {
      const started = typeof source.start === "function"
        ? source.start.call(source, this.__controller)
        : undefined;
      this.__startPromise = Promise.resolve(started);
      this.__startPromise.then(() => {
        this.__started = true;
        this.__pullIfNeeded();
      }, error => this.__error(error));
    } catch (error) {
      this.__startPromise = Promise.reject(error);
      this.__startPromise.catch(() => {});
      this.__error(error);
    }
  }

  get locked() {
    return this.__reader !== null;
  }

  getReader(options = {}) {
    if (options === null || (typeof options !== "object" && typeof options !== "function")) {
      throw new TypeError("ReadableStream reader options must be an object");
    }
    if (this.locked) throw new TypeError("ReadableStream is locked");
    if (options.mode === "byob") {
      if (this.__type !== "bytes") throw new TypeError("BYOB reader requires a byte stream");
      return new ReadableStreamBYOBReader(this, internalToken);
    }
    if (options.mode !== undefined) throw new RangeError("Invalid reader mode");
    return new ReadableStreamDefaultReader(this, internalToken);
  }

  cancel(reason) {
    if (this.locked) throw new TypeError("ReadableStream is locked");
    return this.__cancel(reason);
  }

  pipeTo(destination, options = {}) {
    if (destination === null || (typeof destination !== "object" && typeof destination !== "function")) {
      throw new TypeError("WritableStream destination is required");
    }
    if (options === null || (typeof options !== "object" && typeof options !== "function")) {
      throw new TypeError("pipeTo options must be an object");
    }
    const reader = this.getReader();
    let writer;
    try {
      writer = destination.getWriter();
    } catch (error) {
      reader.releaseLock();
      throw error;
    }
    if (writer === null || (typeof writer !== "object" && typeof writer !== "function")) {
      reader.releaseLock();
      throw new TypeError("WritableStream writer is required");
    }
    reader.closed?.catch(() => {});
    writer.closed?.catch?.(() => {});

    const signal = options.signal;
    let abortHandler;
    let abortPromise;
    if (signal !== undefined && signal !== null) {
      if (typeof signal.addEventListener !== "function") {
        reader.releaseLock();
        writer.releaseLock?.();
        throw new TypeError("pipeTo signal must be an AbortSignal");
      }
      abortPromise = new Promise(resolve => {
        abortHandler = () => resolve(signal.reason ?? new DOMException("The operation was aborted.", "AbortError"));
        if (signal.aborted) abortHandler();
        else signal.addEventListener("abort", abortHandler, { once: true });
      });
    }

    const race = operation => abortPromise
      ? Promise.race([operation, abortPromise.then(reason => { throw reason; })])
      : operation;
    const ignore = operation => {
      try {
        return Promise.resolve(operation()).catch(() => {});
      } catch {
        return Promise.resolve();
      }
    };
    const releaseLocks = () => {
      reader.releaseLock();
      writer.releaseLock?.();
    };
    const run = async () => {
      let failureKind = null;
      try {
        while (true) {
          let result;
          try {
            result = await race(reader.read());
          } catch (error) {
            failureKind = "read";
            throw error;
          }
          if (result.done) break;
          try {
            await race(writer.write(result.value));
          } catch (error) {
            failureKind = "write";
            throw error;
          }
        }
        if (!options.preventClose) {
          try {
            await race(writer.close());
          } catch (error) {
            failureKind = "close";
            throw error;
          }
        }
      } catch (error) {
        const signalAborted = signal?.aborted && (signal.reason === error || error?.name === "AbortError");
        const operations = [];
        if (signalAborted) {
          if (!options.preventAbort) operations.push(ignore(() => writer.abort?.(error)));
          if (!options.preventCancel) operations.push(ignore(() => reader.cancel(error)));
        } else if (failureKind === "read") {
          if (!options.preventAbort) operations.push(ignore(() => writer.abort?.(error)));
        } else if (failureKind === "write") {
          if (!options.preventCancel) operations.push(ignore(() => reader.cancel(error)));
        } else if (failureKind === "close") {
          if (!options.preventCancel) operations.push(ignore(() => reader.cancel(error)));
        } else {
          if (!options.preventAbort) operations.push(ignore(() => writer.abort?.(error)));
          if (!options.preventCancel) operations.push(ignore(() => reader.cancel(error)));
        }
        releaseLocks();
        await Promise.all(operations);
        throw error;
      } finally {
        if (signal && abortHandler) signal.removeEventListener?.("abort", abortHandler);
        releaseLocks();
      }
    };
    return run();
  }

  pipeThrough(transform, options = {}) {
    if (transform === null || typeof transform !== "object") throw new TypeError("TransformStream is required");
    const pipe = this.pipeTo(transform.writable, options);
    pipe.catch(error => transform.readable?.__error?.(error));
    return transform.readable;
  }

  tee() {
    if (this.locked) throw new TypeError("ReadableStream is locked");
    const reader = this.getReader();
    const branches = {
      left: { controller: null, canceled: false, reason: undefined },
      right: { controller: null, canceled: false, reason: undefined },
    };
    let finished = false;
    let pullPromise = null;
    let cancelPromise = null;
    const cancelDeferred = deferred();

    const pump = () => {
      if (pullPromise) return pullPromise;
      if (finished) return Promise.resolve();
      pullPromise = Promise.resolve(reader.read()).then(result => {
        if (result.done) {
          finished = true;
          for (const branch of Object.values(branches)) {
            if (!branch.canceled) branch.controller?.close();
          }
          return;
        }
        for (const branch of Object.values(branches)) {
          if (!branch.canceled) {
            branch.controller?.enqueue(this.__type === "bytes" ? cloneChunk(result.value) : result.value);
          }
        }
      }, error => {
        finished = true;
        for (const branch of Object.values(branches)) {
          if (!branch.canceled) branch.controller?.error(error);
        }
      }).then(() => {
        pullPromise = null;
      });
      return pullPromise;
    };

    const cancelBranch = (name, reason) => {
      const branch = branches[name];
      if (branch.canceled) return cancelPromise ?? Promise.resolve();
      branch.canceled = true;
      branch.reason = reason;
      if (!branches.left.canceled || !branches.right.canceled) return cancelDeferred.promise;
      if (cancelPromise) return cancelPromise;
      const finalReason = branches.right.reason;
      cancelPromise = Promise.resolve(reader.cancel(finalReason)).then(
        value => {
          cancelDeferred.resolve(value);
          return value;
        },
        error => {
          cancelDeferred.reject(error);
          throw error;
        },
      );
      return cancelPromise;
    };

    const createBranch = name => new ReadableStream({
      type: this.__type === "bytes" ? "bytes" : undefined,
      start(controller) {
        branches[name].controller = controller;
      },
      pull() {
        return pump();
      },
      cancel(reason) {
        return cancelBranch(name, reason);
      },
    });
    return [createBranch("left"), createBranch("right")];
  }

  values(options = {}) {
    const reader = this.getReader();
    let finished = false;
    return {
      next: () => {
        if (finished) return Promise.resolve({ done: true, value: undefined });
        return reader.read().then(result => {
          if (result.done) {
            finished = true;
            reader.releaseLock();
          }
          return result;
        }, error => {
          finished = true;
          reader.releaseLock();
          throw error;
        });
      },
      return: async value => {
        if (finished) return { done: true, value };
        finished = true;
        try {
          if (!options.preventCancel) await reader.cancel();
        } finally {
          reader.releaseLock();
        }
        return { done: true, value };
      },
      [Symbol.asyncIterator]() {
        return this;
      },
    };
  }

  [Symbol.asyncIterator]() {
    return this.values();
  }

  __desiredSize() {
    if (this.__state !== "readable") return null;
    return this.__strategy.highWaterMark - this.__queueSize;
  }

  __enqueue(value) {
    if (this.__state !== "readable") throw new TypeError("ReadableStream is not writable");
    if (this.__type === "bytes") {
      if (value instanceof ArrayBuffer) value = new Uint8Array(value);
      if (!ArrayBuffer.isView(value)) throw new TypeError("Byte stream chunks must be ArrayBuffer views");
      value = new Uint8Array(value.buffer, value.byteOffset, value.byteLength);
    }
    let size;
    try {
      const sizeAlgorithm = this.__strategy.size;
      size = Number(sizeAlgorithm(value));
      if (!Number.isFinite(size) || size < 0) throw new RangeError("Invalid chunk size");
    } catch (error) {
      this.__error(error);
      throw error;
    }
    this.__queue.push({ value, size });
    this.__queueSize += size;
    this.__drain();
    this.__pullIfNeeded();
  }

  __close() {
    if (this.__state !== "readable") throw new TypeError("ReadableStream is not readable");
    this.__state = "closed";
    this.__drain();
    this.__reader?.__resolveClosed();
  }

  __error(error) {
    if (this.__state !== "readable") return;
    this.__state = "errored";
    this.__storedError = error;
    this.__queue = [];
    this.__queueSize = 0;
    this.__drain();
    this.__reader?.__rejectClosed(error);
  }

  __cancel(reason) {
    this.__disturbed = true;
    if (this.__state === "closed") return Promise.resolve();
    if (this.__state === "canceled") {
      return this.__cancelPromise ? this.__cancelPromise.then(value => value, error => { throw error; }) : Promise.resolve();
    }
    if (this.__state === "errored") return rejected(this.__storedError);
    this.__state = "canceled";
    this.__queue = [];
    this.__queueSize = 0;
    this.__drain();
    if (this.__cancelPromise) return this.__cancelPromise;
    try {
      const result = typeof this.__source.cancel === "function"
        ? this.__source.cancel.call(this.__source, reason)
        : undefined;
      this.__cancelPromise = Promise.resolve(result);
    } catch (error) {
      this.__cancelPromise = rejected(error);
    }
    this.__reader?.__resolveClosed();
    return this.__cancelPromise;
  }

  __read(view, min = 1) {
    const autoAllocate = view === undefined && this.__type === "bytes" && this.__autoAllocateChunkSize !== undefined;
    if (autoAllocate) view = new Uint8Array(this.__autoAllocateChunkSize);
    const request = new ReadRequest(view, autoAllocate, min);
    const needsMoreBytes = request.view && this.__type === "bytes"
      && request.written + this.__availableBytes() < request.min;
    if (this.__queue.length > 0 && (!needsMoreBytes || this.__state !== "readable")) {
      request.deferred.resolve(this.__dequeue(request));
      this.__pullIfNeeded();
      return request.deferred.promise;
    }
    if (this.__state === "errored") return rejected(this.__storedError);
    if (this.__state === "closed" || this.__state === "canceled") {
      request.deferred.resolve(this.__doneResult(request));
      return request.deferred.promise;
    }
    this.__readRequests.push(request);
    this.__pullIfNeeded();
    return request.deferred.promise;
  }

  __doneResult(request) {
    if (request.view && request.written > 0) {
      return { done: false, value: request.view.subarray(0, request.written) };
    }
    return { done: true, value: request.view && !request.autoAllocate ? request.view.subarray(0, 0) : undefined };
  }

  __dequeue(request) {
    if (!request.view || this.__type !== "bytes") {
      const entry = this.__queue.shift();
      this.__queueSize -= entry.size;
      return { done: false, value: entry.value };
    }
    while (this.__queue.length && request.written < request.view.byteLength) {
      const entry = this.__queue[0];
      const bytes = entry.value instanceof ArrayBuffer
        ? new Uint8Array(entry.value)
        : new Uint8Array(entry.value.buffer, entry.value.byteOffset, entry.value.byteLength);
      const count = Math.min(bytes.byteLength, request.view.byteLength - request.written);
      new Uint8Array(request.view.buffer, request.view.byteOffset + request.written, count)
        .set(bytes.subarray(0, count));
      request.written += count;
      if (count === bytes.byteLength) {
        this.__queue.shift();
        this.__queueSize -= entry.size;
      } else {
        entry.value = bytes.subarray(count);
        entry.size = entry.value.byteLength;
        this.__queueSize -= count;
      }
    }
    return { done: false, value: request.view.subarray(0, request.written) };
  }

  __availableBytes() {
    return this.__queue.reduce((total, entry) => {
      const value = entry.value;
      return total + (value instanceof ArrayBuffer ? value.byteLength : ArrayBuffer.isView(value) ? value.byteLength : 0);
    }, 0);
  }

  __drain() {
    while (this.__readRequests.length && this.__queue.length) {
      const request = this.__readRequests[0];
      if (this.__state === "readable" && request.view && this.__type === "bytes"
        && request.written + this.__availableBytes() < request.min) break;
      this.__readRequests.shift();
      request.deferred.resolve(this.__dequeue(request));
    }
    if (this.__state === "errored") {
      while (this.__readRequests.length) this.__readRequests.shift().deferred.reject(this.__storedError);
    } else if (this.__state === "closed" || this.__state === "canceled") {
      while (this.__readRequests.length) {
        const request = this.__readRequests.shift();
        request.deferred.resolve(this.__doneResult(request));
      }
    }
  }

  __respond(request, bytesWritten) {
    const index = this.__readRequests.indexOf(request);
    if (index < 0 || this.__state !== "readable") throw new TypeError("BYOB request is invalid");
    const count = Number(bytesWritten);
    if (!Number.isInteger(count) || count <= 0 || count > request.view.byteLength - request.written) {
      throw new RangeError("Invalid bytesWritten");
    }
    request.written += count;
    request.byobRequest = null;
    if (request.written >= request.min) {
      this.__readRequests.splice(index, 1);
      request.deferred.resolve({ done: false, value: request.view.subarray(0, request.written) });
    }
    this.__pullIfNeeded();
  }

  __respondWithNewView(request, view) {
    const index = this.__readRequests.indexOf(request);
    if (index < 0 || this.__state !== "readable") throw new TypeError("BYOB request is invalid");
    if (view.buffer !== request.view.buffer
      || view.byteLength === 0
      || view.byteLength > request.view.byteLength - request.written) {
      throw new RangeError("Invalid BYOB view");
    }
    request.written += view.byteLength;
    request.byobRequest = null;
    if (request.written >= request.min) {
      this.__readRequests.splice(index, 1);
      request.deferred.resolve({ done: false, value: view });
    }
    this.__pullIfNeeded();
  }

  __pullIfNeeded() {
    if (!this.__started || this.__state !== "readable") return;
    const needsPull = this.__readRequests.length > 0 || this.__desiredSize() > 0;
    if (!needsPull || typeof this.__source.pull !== "function") return;
    if (this.__pulling) {
      this.__pullAgain = true;
      return;
    }
    this.__pulling = true;
    this.__pullAgain = false;
    try {
      const result = this.__source.pull.call(this.__source, this.__controller);
      Promise.resolve(result).then(() => {
        this.__pulling = false;
        this.__drain();
        if (this.__pullAgain) this.__pullIfNeeded();
      }, error => {
        this.__pulling = false;
        this.__error(error);
      });
    } catch (error) {
      this.__pulling = false;
      this.__error(error);
    }
  }

  static from(iterable) {
    if (iterable === null || iterable === undefined) throw new TypeError("Object is not iterable");
    const asyncMethod = iterable[Symbol.asyncIterator];
    const syncMethod = iterable[Symbol.iterator];
    if (typeof asyncMethod !== "function" && typeof syncMethod !== "function") {
      throw new TypeError("Object is not iterable");
    }
    const iterator = typeof asyncMethod === "function"
      ? asyncMethod.call(iterable)
      : syncMethod.call(iterable);
    let returned = false;
    return new ReadableStream({
      async pull(controller) {
        const result = await iterator.next();
        if (result === null || typeof result !== "object") throw new TypeError("Iterator result is not an object");
        if (result.done) controller.close();
        else controller.enqueue(result.value);
      },
      async cancel() {
        if (!returned) {
          returned = true;
          await iterator.return?.();
        }
      },
    });
  }
}

export class ReadableStreamDefaultReader {
  constructor(stream, token) {
    if (token !== internalToken || !(stream instanceof ReadableStream) || stream.locked) {
      throw new TypeError("Illegal constructor");
    }
    this.__stream = stream;
    this.__released = false;
    this.__closed = deferred();
    this.__closedSettled = false;
    stream.__reader = this;
    if (stream.__state === "errored") this.__rejectClosed(stream.__storedError);
    else if (stream.__state === "closed" || stream.__state === "canceled") this.__resolveClosed();
    stream.__pullIfNeeded();
  }

  get closed() {
    return this.__closed.promise;
  }

  read() {
    if (this.__released) return rejected(new TypeError("ReadableStream reader was released"));
    this.__stream.__disturbed = true;
    return this.__stream.__read();
  }

  cancel(reason) {
    if (this.__released) return rejected(new TypeError("ReadableStream reader was released"));
    return this.__stream.__cancel(reason);
  }

  releaseLock() {
    if (this.__released) return;
    this.__released = true;
    if (this.__stream.__reader === this) {
      this.__stream.__reader = null;
      const error = new TypeError("ReadableStream reader was released");
      while (this.__stream.__readRequests.length) this.__stream.__readRequests.shift().deferred.reject(error);
    }
    if (!this.__closedSettled) this.__rejectClosed(new TypeError("ReadableStream reader was released"));
  }

  __resolveClosed() {
    this.__closed.resolve();
    this.__closedSettled = true;
  }

  __rejectClosed(error) {
    this.__closed.reject(error);
    this.__closedSettled = true;
  }
}

export class ReadableStreamBYOBReader extends ReadableStreamDefaultReader {
  constructor(stream, token) {
    super(stream, token);
  }

  read(view) {
    if (this.__released) return rejected(new TypeError("ReadableStream reader was released"));
    if (!ArrayBuffer.isView(view) || view instanceof DataView || view.byteLength === 0) {
      return rejected(new TypeError("BYOB read requires a non-empty ArrayBufferView"));
    }
    this.__stream.__disturbed = true;
    return this.__stream.__read(view, 1);
  }

  readAtLeast(min, view) {
    if (this.__released) return rejected(new TypeError("ReadableStream reader was released"));
    const minimum = Math.trunc(Number(min));
    if (!Number.isFinite(minimum) || minimum < 1) return rejected(new TypeError("Minimum byte count must be positive"));
    if (!ArrayBuffer.isView(view) || view instanceof DataView || view.byteLength === 0 || minimum > view.byteLength) {
      return rejected(new TypeError("BYOB read requires a view large enough for the minimum"));
    }
    this.__stream.__disturbed = true;
    return this.__stream.__read(view, minimum);
  }
}

export class ReadableStreamDefaultController {
  constructor(token) {
    if (token !== internalToken) throw new TypeError("Illegal constructor");
  }

  get desiredSize() {
    return this.__stream.__desiredSize();
  }

  enqueue(value) {
    return this.__stream.__enqueue(value);
  }

  close() {
    return this.__stream.__close();
  }

  error(error) {
    return this.__stream.__error(error);
  }
}

export class ReadableByteStreamController extends ReadableStreamDefaultController {
  get byobRequest() {
    const request = this.__stream.__readRequests.find(value => value.view);
    if (!request) return null;
    if (!request.byobRequest) {
      request.byobRequest = new ReadableStreamBYOBRequest(
        request.view.subarray(request.written),
        Math.max(1, request.min - request.written),
        bytesWritten => this.__stream.__respond(request, bytesWritten),
        view => this.__stream.__respondWithNewView(request, view),
        internalToken,
      );
    }
    return request.byobRequest;
  }
}

export class ReadableStreamBYOBRequest {
  constructor(view, atLeast, respond, respondWithNewView, token) {
    if (token !== internalToken) throw new TypeError("Illegal constructor");
    this.__view = view;
    this.__atLeast = atLeast;
    this.__respond = respond;
    this.__respondWithNewView = respondWithNewView;
  }

  get view() {
    return this.__view;
  }

  get atLeast() {
    return this.__atLeast;
  }

  respond(bytesWritten) {
    if (typeof this.__respond !== "function") throw new TypeError("BYOB request is invalid");
    return this.__respond(bytesWritten);
  }

  respondWithNewView(view) {
    if (!ArrayBuffer.isView(view)) throw new TypeError("BYOB view is invalid");
    if (typeof this.__respondWithNewView !== "function") throw new TypeError("BYOB request is invalid");
    return this.__respondWithNewView(view);
  }
}

export class WritableStream {
  constructor(sink = {}, strategy = {}) {
    if (sink === null || (typeof sink !== "object" && typeof sink !== "function")) {
      throw new TypeError("WritableStream sink must be an object");
    }
    this.__sink = sink;
    this.__strategy = normalizeStrategy(strategy, 1);
    this.__state = "writable";
    this.__storedError = undefined;
    this.__queue = [];
    this.__queueSize = 0;
    this.__current = null;
    this.__writer = null;
    this.__started = false;
    this.__startPromise = Promise.resolve();
    this.__closeRequest = null;
    this.__abortPromise = null;
    this.__abortController = typeof globalThis.AbortController === "function"
      ? new globalThis.AbortController()
      : null;
    this.__signal = this.__abortController?.signal ?? { aborted: false, reason: undefined };
    this.__controller = Object.create(WritableStreamDefaultController.prototype);
    Object.defineProperty(this.__controller, "__stream", { value: this });
    try {
      const started = typeof sink.start === "function" ? sink.start.call(sink, this.__controller) : undefined;
      this.__startPromise = Promise.resolve(started);
      this.__startPromise.then(() => {
        this.__started = true;
        this.__process();
      }, error => this.__error(error));
    } catch (error) {
      this.__startPromise = Promise.reject(error);
      this.__startPromise.catch(() => {});
      this.__error(error);
    }
  }

  get locked() {
    return this.__writer !== null;
  }

  getWriter() {
    if (this.locked) throw new TypeError("WritableStream is locked");
    return new WritableStreamDefaultWriter(this, internalToken);
  }

  abort(reason) {
    if (this.locked) return rejected(new TypeError("WritableStream is locked"));
    return this.__abort(reason);
  }

  close() {
    if (this.locked) return rejected(new TypeError("WritableStream is locked"));
    return this.__close();
  }

  __desiredSize() {
    if (this.__state === "errored" || this.__state === "closed") return null;
    return this.__strategy.highWaterMark - this.__queueSize;
  }

  __write(value) {
    if (this.__state !== "writable") {
      return rejected(this.__storedError ?? new TypeError("WritableStream is closed"));
    }
    let size;
    try {
      const sizeAlgorithm = this.__strategy.size;
      size = Number(sizeAlgorithm(value));
      if (!Number.isFinite(size) || size < 0) throw new RangeError("Invalid chunk size");
    } catch (error) {
      this.__error(error);
      return rejected(error);
    }
    const request = { value, size, deferred: deferred() };
    this.__queue.push(request);
    this.__queueSize += size;
    this.__updateReady();
    this.__process();
    return request.deferred.promise;
  }

  __close() {
    if (this.__state === "closing") return this.__closeRequest.promise;
    if (this.__state !== "writable") {
      return rejected(this.__storedError ?? new TypeError("WritableStream is closed"));
    }
    this.__state = "closing";
    this.__closeRequest = deferred();
    this.__updateReady();
    this.__process();
    return this.__closeRequest.promise;
  }

  __abort(reason) {
    if (this.__state === "closed") return Promise.resolve();
    if (this.__abortPromise) return this.__abortPromise;
    if (this.__state === "errored") return rejected(this.__storedError);
    this.__state = "errored";
    this.__storedError = reason;
    this.__abortController?.abort(reason);
    this.__rejectPending(reason);
    this.__updateReady();
    this.__abortPromise = this.__startPromise.then(() => (
      typeof this.__sink.abort === "function" ? this.__sink.abort.call(this.__sink, reason) : undefined
    )).catch(error => {
      this.__storedError = error;
      throw error;
    });
    this.__writer?.__rejectClosed(this.__storedError);
    return this.__abortPromise;
  }

  __error(error) {
    if (this.__state === "errored" || this.__state === "closed") return;
    this.__state = "errored";
    this.__storedError = error;
    this.__rejectPending(error);
    this.__updateReady();
    this.__writer?.__rejectClosed(error);
  }

  __rejectPending(error) {
    if (this.__current && !this.__current.close) this.__current.deferred.reject(error);
    while (this.__queue.length) this.__queue.shift().deferred.reject(error);
    this.__queueSize = 0;
    this.__closeRequest?.reject(error);
  }

  __updateReady() {
    const writer = this.__writer;
    if (!writer) return;
    if (this.__state === "closed") {
      writer.__ready.resolve();
      writer.__readySettled = true;
      return;
    }
    if (this.__state === "errored") {
      if (writer.__readySettled) {
        writer.__ready = deferred();
        writer.__readySettled = false;
      }
      writer.__ready.reject(this.__storedError);
      writer.__readySettled = true;
      return;
    }
    if (this.__desiredSize() > 0) {
      writer.__ready.resolve();
      writer.__readySettled = true;
    } else if (writer.__readySettled) {
      writer.__ready = deferred();
      writer.__readySettled = false;
    }
  }

  __process() {
    if (!this.__started || this.__state === "errored" || this.__state === "closed" || this.__current) return;
    const request = this.__queue.shift();
    if (request) {
      this.__current = request;
      let result;
      try {
        result = typeof this.__sink.write === "function"
          ? this.__sink.write.call(this.__sink, request.value, this.__controller)
          : undefined;
      } catch (error) {
        this.__current = null;
        this.__queueSize -= request.size;
        request.deferred.reject(error);
        this.__error(error);
        return;
      }
      Promise.resolve(result).then(() => {
        if (this.__current === request) {
          this.__current = null;
          this.__queueSize -= request.size;
        }
        if (this.__state === "errored") request.deferred.reject(this.__storedError);
        else request.deferred.resolve();
        this.__updateReady();
        this.__process();
      }, error => {
        if (this.__current === request) {
          this.__current = null;
          this.__queueSize -= request.size;
        }
        request.deferred.reject(error);
        this.__error(error);
      });
      return;
    }
    if (this.__state !== "closing" || !this.__closeRequest) return;
    let result;
    try {
      result = typeof this.__sink.close === "function" ? this.__sink.close.call(this.__sink) : undefined;
    } catch (error) {
      this.__error(error);
      return;
    }
    this.__current = { close: true };
    Promise.resolve(result).then(() => {
      this.__current = null;
      if (this.__state === "errored") return;
      this.__state = "closed";
      this.__closeRequest.resolve();
      this.__writer?.__resolveClosed();
      this.__updateReady();
    }, error => {
      this.__current = null;
      this.__error(error);
    });
  }
}

export class WritableStreamDefaultWriter {
  constructor(stream, token) {
    if (token !== internalToken || !(stream instanceof WritableStream) || stream.locked) {
      throw new TypeError("Illegal constructor");
    }
    this.__stream = stream;
    this.__released = false;
    this.__closed = deferred();
    this.__closedSettled = false;
    this.__ready = deferred();
    this.__readySettled = false;
    stream.__writer = this;
    if (stream.__state === "errored") this.__rejectClosed(stream.__storedError);
    else if (stream.__state === "closed") this.__resolveClosed();
    if (stream.__desiredSize() > 0) {
      this.__ready.resolve();
      this.__readySettled = true;
    }
  }

  get closed() {
    return this.__closed.promise;
  }

  get ready() {
    return this.__ready.promise;
  }

  get desiredSize() {
    if (this.__released) throw new TypeError("WritableStream writer was released");
    return this.__stream.__desiredSize();
  }

  write(chunk) {
    if (this.__released) return rejected(new TypeError("WritableStream writer was released"));
    return this.__stream.__write(chunk);
  }

  close() {
    if (this.__released) return rejected(new TypeError("WritableStream writer was released"));
    return this.__stream.__close();
  }

  abort(reason) {
    if (this.__released) return rejected(new TypeError("WritableStream writer was released"));
    return this.__stream.__abort(reason);
  }

  releaseLock() {
    if (this.__released) return;
    this.__released = true;
    if (this.__stream.__writer === this) this.__stream.__writer = null;
    const error = new TypeError("WritableStream writer was released");
    if (!this.__closedSettled) this.__closed.reject(error);
    if (this.__readySettled) this.__ready = deferred();
    this.__ready.reject(error);
    this.__readySettled = true;
  }

  __resolveClosed() {
    this.__closed.resolve();
    this.__closedSettled = true;
  }

  __rejectClosed(error) {
    this.__closed.reject(error);
    this.__closedSettled = true;
  }
}

export class WritableStreamDefaultController {
  constructor(token) {
    if (token !== internalToken) throw new TypeError("Illegal constructor");
  }

  get signal() {
    return this.__stream.__signal;
  }

  error(error) {
    this.__stream.__error(error);
  }
}

export class TransformStreamDefaultController {
  constructor(readableController, token) {
    if (token !== internalToken) throw new TypeError("Illegal constructor");
    this.__readableController = readableController;
    this.__writable = null;
  }

  get desiredSize() {
    return this.__readableController.desiredSize;
  }

  enqueue(chunk) {
    return this.__readableController.enqueue(chunk);
  }

  error(error) {
    this.__readableController.error(error);
    this.__writable?.__error(error);
  }

  terminate() {
    this.__readableController.close();
    this.__writable?.__error(new TypeError("TransformStream has been terminated"));
  }
}

export class TransformStream {
  constructor(transformer = {}, writableStrategy = {}, readableStrategy = {}) {
    if (transformer === null || (typeof transformer !== "object" && typeof transformer !== "function")) {
      throw new TypeError("TransformStream transformer must be an object");
    }
    let transformController;
    let writable;
    const readable = new ReadableStream({
      start(controller) {
        transformController = new TransformStreamDefaultController(controller, internalToken);
        return typeof transformer.start === "function"
          ? transformer.start.call(transformer, transformController)
          : undefined;
      },
      cancel(reason) {
        writable?.__error(reason);
      },
    }, readableStrategy);
    writable = new WritableStream({
      start() {
        return readable.__startPromise;
      },
      write(chunk) {
        try {
          const result = typeof transformer.transform === "function"
            ? transformer.transform.call(transformer, chunk, transformController)
            : transformController.enqueue(chunk);
          return Promise.resolve(result).catch(error => {
            transformController.error(error);
            throw error;
          });
        } catch (error) {
          transformController.error(error);
          throw error;
        }
      },
      close() {
        try {
          const result = typeof transformer.flush === "function"
            ? transformer.flush.call(transformer, transformController)
            : undefined;
          return Promise.resolve(result).then(() => {
            if (readable.__state === "readable") readable.__controller.close();
          }).catch(error => {
            transformController.error(error);
            throw error;
          });
        } catch (error) {
          transformController.error(error);
          throw error;
        }
      },
      abort(reason) {
        transformController.error(reason);
      },
    }, writableStrategy);
    transformController.__writable = writable;
    if (readable.__state === "errored") writable.__error(readable.__storedError);
    this.__readable = readable;
    this.__writable = writable;
  }

  get readable() {
    return this.__readable;
  }

  get writable() {
    return this.__writable;
  }
}

export class TextEncoderStream extends TransformStream {
  constructor() {
    super({ transform(chunk, controller) {
      controller.enqueue(new globalThis.TextEncoder().encode(String(chunk)));
    } });
  }
  get encoding() { return "utf-8"; }
}

export class TextDecoderStream extends TransformStream {
  constructor(label = "utf-8", options = {}) {
    const decoder = new globalThis.TextDecoder(label, options);
    super({
      transform(chunk, controller) {
        const bytes = chunk instanceof ArrayBuffer
          ? new Uint8Array(chunk)
          : ArrayBuffer.isView(chunk)
            ? new Uint8Array(chunk.buffer, chunk.byteOffset, chunk.byteLength)
            : null;
        if (bytes === null) throw new TypeError("TextDecoderStream accepts byte chunks");
        const value = decoder.decode(bytes, { stream: true });
        if (value !== "") controller.enqueue(value);
      },
      flush(controller) {
        const value = decoder.decode();
        if (value !== "") controller.enqueue(value);
      },
    });
    Object.defineProperties(this, {
      __encoding: { configurable: true, enumerable: false, value: decoder.encoding },
      __fatal: { configurable: true, enumerable: false, value: decoder.fatal },
      __ignoreBOM: { configurable: true, enumerable: false, value: decoder.ignoreBOM },
    });
  }
  get encoding() { return this.__encoding; }
  get fatal() { return this.__fatal; }
  get ignoreBOM() { return this.__ignoreBOM; }
}

class CompressionTransform {
  constructor(format, decoder) {
    if (!["deflate", "deflate-raw", "gzip"].includes(format)) throw new TypeError("Unsupported compression format");
    this.__format = format;
    this.__decoder = decoder;
    this.__chunks = [];
  }

  transform(chunk) {
    if (!ArrayBuffer.isView(chunk) && !(chunk instanceof ArrayBuffer)) {
      throw new TypeError("Compression streams accept byte chunks");
    }
    this.__chunks.push(
      chunk instanceof ArrayBuffer
        ? new Uint8Array(chunk)
        : new Uint8Array(chunk.buffer, chunk.byteOffset, chunk.byteLength),
    );
  }

  async finish(controller) {
    const size = this.__chunks.reduce((total, chunk) => total + chunk.byteLength, 0);
    const input = new Uint8Array(size);
    let offset = 0;
    for (const chunk of this.__chunks) {
      input.set(chunk, offset);
      offset += chunk.byteLength;
    }
    const { compress, decompress } = await import("tokamak:host");
    const output = (this.__decoder ? decompress : compress)(this.__format, input);
    if (output.byteLength > 0) controller.enqueue(output);
  }
}

export class CompressionStream extends TransformStream {
  constructor(format) {
    const state = new CompressionTransform(format, false);
    super({ transform: chunk => state.transform(chunk), flush: controller => state.finish(controller) });
  }
}

export class DecompressionStream extends TransformStream {
  constructor(format) {
    const state = new CompressionTransform(format, true);
    super({ transform: chunk => state.transform(chunk), flush: controller => state.finish(controller) });
  }
}

export class IdentityTransformStream extends TransformStream {
  constructor() {
    super({ transform(chunk, controller) { controller.enqueue(chunk); } });
  }
}

export class FixedLengthStream {
  constructor(length) {
    const expected = Number(length);
    if (!Number.isSafeInteger(expected) || expected < 0) throw new RangeError("Invalid fixed stream length");
    let total = 0;
    const transform = new TransformStream({
      transform(chunk, controller) {
        const value = chunk instanceof ArrayBuffer
          ? new Uint8Array(chunk)
          : ArrayBuffer.isView(chunk)
            ? new Uint8Array(chunk.buffer, chunk.byteOffset, chunk.byteLength)
            : null;
        if (value === null) {
          controller.error(new TypeError("FixedLengthStream accepts byte chunks"));
          return;
        }
        total += value.byteLength;
        if (total > expected) {
          controller.error(new TypeError("FixedLengthStream received too many bytes"));
          return;
        }
        controller.enqueue(value);
      },
      flush(controller) {
        if (total !== expected) controller.error(new TypeError("FixedLengthStream received the wrong number of bytes"));
      },
    });
    this.readable = transform.readable;
    this.writable = transform.writable;
  }
}

export class CountQueuingStrategy {
  constructor({ highWaterMark }) {
    Object.defineProperty(this, "__highWaterMark", { configurable: true, enumerable: false, value: Number(highWaterMark) });
  }

  get highWaterMark() { return this.__highWaterMark; }
  size() {
    return 1;
  }
}

export class ByteLengthQueuingStrategy {
  constructor({ highWaterMark }) {
    Object.defineProperty(this, "__highWaterMark", { configurable: true, enumerable: false, value: Number(highWaterMark) });
  }

  get highWaterMark() { return this.__highWaterMark; }
  size(chunk) {
    return chunk.byteLength;
  }
}

function exposeProperties(prototype, names) {
  for (const name of names) {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    if (descriptor) Object.defineProperty(prototype, name, { ...descriptor, enumerable: true });
  }
}

function copyProperties(prototype, source, names) {
  for (const name of names) {
    const descriptor = Object.getOwnPropertyDescriptor(source, name);
    if (descriptor) Object.defineProperty(prototype, name, { ...descriptor, enumerable: true });
  }
}

exposeProperties(ReadableStream.prototype, ["locked", "getReader", "cancel", "pipeTo", "pipeThrough", "tee", "values"]);
exposeProperties(ReadableStreamDefaultReader.prototype, ["closed", "read", "cancel", "releaseLock"]);
exposeProperties(ReadableStreamBYOBReader.prototype, ["read", "readAtLeast"]);
copyProperties(ReadableStreamBYOBReader.prototype, ReadableStreamDefaultReader.prototype, ["closed", "cancel", "releaseLock"]);
exposeProperties(ReadableStreamDefaultController.prototype, ["desiredSize", "enqueue", "close", "error"]);
copyProperties(ReadableByteStreamController.prototype, ReadableStreamDefaultController.prototype, ["desiredSize", "enqueue", "close", "error"]);
exposeProperties(ReadableByteStreamController.prototype, ["byobRequest"]);
exposeProperties(ReadableStreamBYOBRequest.prototype, ["view", "atLeast", "respond", "respondWithNewView"]);
exposeProperties(WritableStream.prototype, ["locked", "getWriter", "abort", "close"]);
exposeProperties(WritableStreamDefaultWriter.prototype, ["closed", "ready", "desiredSize", "write", "close", "abort", "releaseLock"]);
exposeProperties(WritableStreamDefaultController.prototype, ["signal", "error"]);
exposeProperties(TransformStreamDefaultController.prototype, ["desiredSize", "enqueue", "error", "terminate"]);
exposeProperties(TransformStream.prototype, ["readable", "writable"]);

export default {
  ReadableStream,
  ReadableStreamBYOBReader,
  ReadableStreamBYOBRequest,
  ReadableByteStreamController,
  ReadableStreamDefaultController,
  ReadableStreamDefaultReader,
  WritableStream,
  WritableStreamDefaultController,
  WritableStreamDefaultWriter,
  TransformStream,
  TransformStreamDefaultController,
  TextEncoderStream,
  TextDecoderStream,
  CompressionStream,
  DecompressionStream,
  CountQueuingStrategy,
  ByteLengthQueuingStrategy,
  FixedLengthStream,
  IdentityTransformStream,
};
