import { markHostObject } from "../globals/objects.mjs";
import { createCompression } from "tokamak:host";
import { structuredClone } from "../globals/structured-clone.mjs";
import { bufferSourceBytes, sharedBufferSourceBytes, string } from "../globals/conversions.mjs";
import { TextDecoder, TextEncoder } from "./text.mjs";
import {
  ReadableStream, ReadableStreamBYOBReader, ReadableStreamBYOBRequest,
  ReadableByteStreamController, ReadableStreamDefaultController, ReadableStreamDefaultReader,
  WritableStream, WritableStreamDefaultController, WritableStreamDefaultWriter,
  setNativeCancelError,
  TransformStream as StandardTransformStream, TransformStreamDefaultController, CountQueuingStrategy, ByteLengthQueuingStrategy,
} from "./web-standard.mjs";
export {
  ReadableStream, ReadableStreamBYOBReader, ReadableStreamBYOBRequest,
  ReadableByteStreamController, ReadableStreamDefaultController, ReadableStreamDefaultReader,
  WritableStream, WritableStreamDefaultController, WritableStreamDefaultWriter,
  TransformStreamDefaultController, CountQueuingStrategy, ByteLengthQueuingStrategy,
};

delete WritableStreamDefaultController.prototype.abortReason;
const nativePair = Symbol("native-transform-pair");
const streamLengths = new WeakMap();

// Record that `stream` delivers exactly `length` bytes.
export function setStreamLength(stream, length) {
  streamLengths.set(stream, length);
  return stream;
}

// The number of bytes `stream` delivers, when known.
export function streamLength(stream) {
  return streamLengths.get(stream);
}

const encoder = new TextEncoder();

// The bytes of a chunk a byte stream's writable side takes: a string or an [AllowShared] BufferSource.
export function writtenChunkBytes(chunk) {
  const bytes = typeof chunk === "string" ? encoder.encode(chunk) : sharedBufferSourceBytes(chunk);
  if (bytes) return bytes;
  throw new TypeError("This TransformStream is being used as a byte stream, but received an object of non-ArrayBuffer/ArrayBufferView type on its writable side.");
}

// The bytes of a chunk read from a stream to drain it: a string or a BufferSource.
export function drainedChunkBytes(chunk) {
  const bytes = typeof chunk === "string" ? encoder.encode(chunk) : bufferSourceBytes(chunk);
  if (bytes) return bytes;
  throw new TypeError("Draining read encountered a value that cannot be converted to bytes");
}

// Both branches of a tee deliver the bytes of the stream they split.
const teeBranches = ReadableStream.prototype.tee;
Object.defineProperty(ReadableStream.prototype, "tee", {
  ...Object.getOwnPropertyDescriptor(ReadableStream.prototype, "tee"),
  value: {
    tee() {
      const branches = teeBranches.call(this);
      const length = streamLengths.get(this);
      if (length !== undefined) for (const branch of branches) streamLengths.set(branch, length);
      return branches;
    },
  }.tee,
});

export function nativeStreamError(reason) {
  if (reason !== null && typeof reason === "object") {
    try { return structuredClone(reason); }
    catch { return new Error("#<Object>"); }
  }
  return new Error(reason === undefined ? "Stream was cancelled." : String(reason));
}

export function nativeReadableStream(source) {
  const stream = new ReadableStream(source);
  setNativeCancelError(stream, nativeStreamError);
  return stream;
}

export class TransformStream {
  #pair;
  constructor(transformer, writableStrategy, readableStrategy) {
    markHostObject(this);
    this.#pair = transformer === nativePair ? writableStrategy : new StandardTransformStream(transformer, writableStrategy, readableStrategy);
  }
  get readable() { return this.#pair.readable; }
  get writable() { return this.#pair.writable; }
}
for (const name of ["readable", "writable"]) {
  Object.defineProperty(TransformStream.prototype, name, { ...Object.getOwnPropertyDescriptor(TransformStream.prototype, name), enumerable: true });
}

export function isDisturbed(stream) { return Boolean(stream?._disturbed); }

Object.defineProperty(ReadableStreamBYOBReader.prototype, "readAtLeast", {
  configurable: true, enumerable: true, writable: true,
  value(minimum, view) {
    minimum = Math.trunc(Number(minimum));
    if (!Number.isFinite(minimum) || minimum < 1) return Promise.reject(new TypeError("Minimum byte count must be positive"));
    if (!ArrayBuffer.isView(view) || minimum > view.byteLength) return Promise.reject(new TypeError("BYOB read requires a view large enough for the minimum"));
    return this.read(view, { min: Math.ceil(minimum / (view.BYTES_PER_ELEMENT ?? 1)) });
  },
});

Object.defineProperty(ReadableStreamBYOBRequest.prototype, "atLeast", {
  configurable: true, enumerable: true,
  get() {
    const pending = this._associatedReadableByteStreamController?._pendingPullIntos.peek();
    return pending ? Math.max(1, pending.minimumFill - pending.bytesFilled) : null;
  },
});

export class TextEncoderStream extends TransformStream {
  constructor() {
    let pending = "";
    super({
      transform(chunk, controller) {
        let text = pending + string(chunk);
        pending = "";
        const last = text.charCodeAt(text.length - 1);
        if (last >= 0xd800 && last <= 0xdbff) {
          pending = text.slice(-1);
          text = text.slice(0, -1);
        }
        if (text) controller.enqueue(encoder.encode(text));
      },
      flush(controller) { if (pending) controller.enqueue(encoder.encode(pending)); },
    });
  }
  get encoding() { return "utf-8"; }
}

export class TextDecoderStream extends TransformStream {
  #decoder;
  constructor(label = "utf-8", options = {}) {
    const decoder = new TextDecoder(label, options);
    super({
      transform(chunk, controller) {
        const bytes = bufferSourceBytes(chunk);
        if (!bytes) throw new TypeError("This TransformStream is being used as a byte stream, but received a value that is not a BufferSource.");
        const value = decoder.decode(bytes, { stream: true });
        if (value !== "") controller.enqueue(value);
      },
      flush(controller) {
        const value = decoder.decode();
        if (value !== "") controller.enqueue(value);
      },
    });
    this.#decoder = decoder;
  }
  get encoding() { return this.#decoder.encoding; }
  get fatal() { return this.#decoder.fatal; }
  get ignoreBOM() { return this.#decoder.ignoreBOM; }
}

function compressionPair(format, decode) {
  let codec = createCompression(String(format), decode);
  let readableController;
  let writableController;
  const readable = nativeReadableStream({
    type: "bytes",
    start(controller) { readableController = controller; },
    cancel(reason) { codec = null; writableController.error(reason); },
  });
  const process = (chunk, finish) => {
    try {
      chunk = typeof chunk === "string" ? encoder.encode(chunk) : sharedBufferSourceBytes(chunk);
      if (!chunk) throw new TypeError("This WritableStream only supports writing byte types.");
      codec(chunk, finish, output => readableController.enqueue(output));
      if (finish) codec = null;
    } catch (error) {
      codec = null;
      readableController.error(error);
      throw error;
    }
  };
  const writable = new WritableStream({
    start(controller) { writableController = controller; },
    write(chunk) { process(chunk, false); },
    close() {
      process(new Uint8Array(), true);
      readableController.close();
      readableController.byobRequest?.respond(0);
    },
    abort(reason) { codec = null; readableController.error(reason); },
  });
  return { readable, writable };
}

export class CompressionStream extends TransformStream {
  constructor(format) { super(nativePair, compressionPair(format, false)); }
}

export class DecompressionStream extends TransformStream {
  constructor(format) { super(nativePair, compressionPair(format, true)); }
}

function identityPair(remaining, strategy = {}) {
  if (strategy === null) strategy = {};
  if (typeof strategy !== "object") throw new TypeError("Expected a queuing strategy");
  let readableController;
  let writableController;
  let available = Promise.withResolvers();
  let pending;
  let ended = false;
  const fail = reason => {
    ended = true;
    pending?.completion.reject(reason);
    pending = undefined;
    available.resolve();
    readableController.error(reason);
    writableController.error(reason);
  };
  const readable = nativeReadableStream({
    type: "bytes",
    start(controller) { readableController = controller; },
    async pull() {
      await available.promise;
      if (ended) return;
      const { bytes, completion } = pending;
      const request = readableController.byobRequest;
      if (request) {
        const count = Math.min(request.view.byteLength, bytes.byteLength - pending.offset);
        request.view.set(bytes.subarray(pending.offset, pending.offset + count));
        pending.offset += count;
        request.respond(count);
      } else {
        const unread = bytes.subarray(pending.offset);
        pending.offset = bytes.byteLength;
        readableController.enqueue(unread);
      }
      if (pending.offset === pending.length) {
        pending = undefined;
        available = Promise.withResolvers();
        completion.resolve();
      }
    },
    cancel: fail,
  });
  const writable = new WritableStream({
    start(controller) {
      writableController = controller;
      controller.signal.addEventListener("abort", () => fail(controller.signal.reason), { once: true });
    },
    write(chunk) {
      try {
        const bytes = writtenChunkBytes(chunk).slice();
        if (bytes.byteLength === 0) return;
        if (remaining !== undefined) {
          remaining -= BigInt(bytes.byteLength);
          if (remaining < 0n) throw new TypeError("Attempt to write too many bytes through a FixedLengthStream.");
        }
        const completion = Promise.withResolvers();
        pending = { bytes, offset: 0, length: bytes.byteLength, completion };
        available.resolve();
        return completion.promise;
      } catch (error) { fail(error); throw error; }
    },
    close() {
      if (remaining !== undefined && remaining !== 0n) {
        const error = new TypeError("FixedLengthStream did not see all expected bytes before close().");
        fail(error);
        throw error;
      }
      readableController.close();
      readableController.byobRequest?.respond(0);
      ended = true;
      available.resolve();
    },
    abort: fail,
  }, strategy.highWaterMark === undefined ? {} : {
    highWaterMark: strategy.highWaterMark,
    size: chunk => typeof chunk === "string" ? chunk.length * 3 : chunk.byteLength ?? 1,
  });
  return { readable, writable };
}

export class IdentityTransformStream extends TransformStream {
  constructor(strategy, length) {
    super(nativePair, strategy === nativePair ? identityPair(length.remaining, length.strategy) : identityPair(undefined, strategy));
  }
}

export class FixedLengthStream extends IdentityTransformStream {
  constructor(length, strategy) {
    let remaining;
    try { remaining = typeof length === "bigint" ? length : BigInt(Math.trunc(Number(length))); }
    catch { throw new TypeError("Expected a non-negative length fitting in uint64"); }
    if (remaining < 0n || remaining > 0xffffffffffffffffn) throw new TypeError("Expected a non-negative length fitting in uint64");
    super(nativePair, { remaining, strategy });
    setStreamLength(this.readable, Number(remaining));
  }
}

for (const Type of [TransformStream, CompressionStream, DecompressionStream, IdentityTransformStream, FixedLengthStream]) {
  Object.defineProperty(Type.prototype, Symbol.toStringTag, { value: Type.name, configurable: true });
}

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
