function abortError(reason) {
  const error = new Error("The operation was aborted");
  error.name = "AbortError";
  error.code = "ABORT_ERR";
  if (reason !== undefined) error.cause = reason;
  return error;
}

function typeError(code, message) {
  return Object.assign(new TypeError(message), { code });
}

function remove(stream, name, listener) {
  stream.removeListener?.(name, listener);
}

function streamKinds(stream, options) {
  const readable = options.readable ?? (stream.readable !== undefined
    ? stream.readable !== false
    : typeof stream.read === "function" || typeof stream.push === "function");
  const writable = options.writable ?? (stream.writable !== undefined
    ? stream.writable !== false
    : typeof stream.write === "function" || typeof stream.end === "function");
  if (!readable && !writable) throw typeError("ERR_INVALID_ARG_TYPE", "The stream is neither readable nor writable");
  return { readable, writable };
}

function finishedWeb(stream, options) {
  return new Promise((resolve, reject) => {
    let handle;
    let abort;
    let settled = false;
    const settle = error => {
      if (settled) return;
      settled = true;
      abort?.();
      handle?.releaseLock?.();
      if (error && options.error !== false) reject(error); else resolve();
    };
    try {
      handle = typeof stream.getReader === "function" ? stream.getReader() : stream.getWriter();
      handle.closed.then(() => settle(), settle);
      const signal = options.signal;
      if (signal) {
        const onAbort = () => settle(abortError(signal.reason));
        if (signal.aborted) onAbort();
        else {
          signal.addEventListener("abort", onAbort, { once: true });
          abort = () => signal.removeEventListener("abort", onAbort);
        }
      }
    } catch (error) { settle(error); }
  });
}

export function finished(stream, options = {}) {
  if (options !== null && (typeof options !== "object" && typeof options !== "function")) {
    return Promise.reject(typeError("ERR_INVALID_ARG_TYPE", "The \"options\" argument must be an object"));
  }
  options ??= {};
  if (options.cleanup !== undefined && typeof options.cleanup !== "boolean") {
    return Promise.reject(typeError("ERR_INVALID_ARG_TYPE", "The \"cleanup\" argument must be of type boolean"));
  }
  if (typeof stream?.getReader === "function" || typeof stream?.getWriter === "function") return finishedWeb(stream, options);
  if (typeof stream?.on !== "function") {
    return Promise.reject(typeError("ERR_INVALID_ARG_TYPE", "The \"stream\" argument must be a Node stream"));
  }

  let kinds;
  try { kinds = streamKinds(stream, options); }
  catch (error) { return Promise.reject(error); }
  const { readable, writable } = kinds;
  return new Promise((resolve, reject) => {
    let readableDone = !readable || stream.readableEnded === true || stream.readable === false;
    let writableDone = !writable || stream.writableFinished === true || stream.writable === false;
    let settled = false;
    let abort;
    const onEnd = () => { readableDone = true; check(); };
    const onFinish = () => { writableDone = true; check(); };
    const onError = error => { if (options.error !== false) settle(error); };
    const onClose = () => {
      if (readableDone && writableDone) settle();
      else settle(Object.assign(new Error("Premature close"), { code: "ERR_STREAM_PREMATURE_CLOSE" }));
    };
    const cleanup = () => {
      if (readable) remove(stream, "end", onEnd);
      if (writable) remove(stream, "finish", onFinish);
      remove(stream, "error", onError);
      remove(stream, "close", onClose);
      abort?.();
    };
    const settle = error => {
      if (settled) return;
      settled = true;
      if (options.cleanup) cleanup();
      if (error && options.error !== false) reject(error); else resolve();
    };
    const check = () => { if (readableDone && writableDone) settle(); };

    if (readable) stream.on("end", onEnd);
    if (writable) stream.on("finish", onFinish);
    stream.on("error", onError);
    stream.on("close", onClose);
    if (options.signal) {
      const onAbort = () => {
        cleanup();
        settle(abortError(options.signal.reason));
      };
      if (options.signal.aborted) onAbort();
      else {
        options.signal.addEventListener("abort", onAbort, { once: true });
        abort = () => options.signal.removeEventListener("abort", onAbort);
      }
    }
    check();
  });
}

function isOptions(value) {
  return value !== null && typeof value === "object"
    && typeof value.on !== "function" && typeof value.pipe !== "function"
    && typeof value.write !== "function" && typeof value.end !== "function"
    && typeof value.getReader !== "function" && typeof value.getWriter !== "function"
    && typeof value[Symbol.asyncIterator] !== "function" && typeof value[Symbol.iterator] !== "function";
}

function destroy(streams) {
  for (const stream of streams) {
    try { stream.destroy?.(); } catch {}
  }
}

async function nodePipeline(streams, options) {
  if (options.signal?.aborted) {
    destroy(streams);
    throw abortError();
  }
  const completionTarget = options.end === false ? streams.at(-2) : streams.at(-1);
  const completion = finished(completionTarget, { cleanup: true });
  let fail;
  let abort;
  const errors = new Promise((_, reject) => {
    fail = reject;
    for (const stream of streams) stream.on?.("error", fail);
  });
  try {
    for (let index = 0; index + 1 < streams.length; index += 1) {
      const source = streams[index];
      const destination = streams[index + 1];
      if (typeof source.pipe !== "function") throw new TypeError("The stream must support pipe()");
      if (index === streams.length - 2 && options.end === false
        && typeof source.on === "function" && typeof destination.write === "function") {
        source.on("data", chunk => destination.write(chunk));
      } else {
        const pipeOptions = options.end === undefined ? undefined : { end: options.end };
        source.pipe(destination, pipeOptions);
      }
    }
    if (options.signal) {
      const onAbort = () => {
        fail(abortError());
        destroy(streams);
      };
      if (options.signal.aborted) onAbort();
      else {
        options.signal.addEventListener("abort", onAbort, { once: true });
        abort = () => options.signal.removeEventListener("abort", onAbort);
      }
    }
    await Promise.race([completion, errors]);
  } catch (error) {
    destroy(streams);
    throw error;
  } finally {
    abort?.();
    for (const stream of streams) remove(stream, "error", fail);
  }
}

export async function pipeline(...args) {
  let options = {};
  if (isOptions(args.at(-1))) options = args.pop();
  if (args.length < 2) throw typeError("ERR_MISSING_ARGS", "The \"streams\" argument must be specified");
  await nodePipeline(args, options);
}

const promises = { finished, pipeline };
export { promises };
export default promises;
