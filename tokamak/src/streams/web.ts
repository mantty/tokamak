import { isObject, markHostObject } from "../globals/objects.ts";
import { createCompression, type CompressionCodec } from "tokamak:host";
import { structuredClone } from "../globals/structured-clone.ts";
import { bufferSourceBytes, sharedBufferSourceBytes, string } from "../globals/conversions.ts";
import { TextDecoder, TextEncoder } from "./text.ts";
import {
  ReadableStream, ReadableStreamBYOBReader, ReadableStreamBYOBRequest,
  ReadableByteStreamController, ReadableStreamDefaultController, ReadableStreamDefaultReader,
  WritableStream, WritableStreamDefaultController, WritableStreamDefaultWriter,
  setNativeCancelError,
  TransformStream as StandardTransformStream, TransformStreamDefaultController, CountQueuingStrategy, ByteLengthQueuingStrategy,
} from "./web-standard.js";
export {
  ReadableStream, ReadableStreamBYOBReader, ReadableStreamBYOBRequest,
  ReadableByteStreamController, ReadableStreamDefaultController, ReadableStreamDefaultReader,
  WritableStream, WritableStreamDefaultController, WritableStreamDefaultWriter,
  TransformStreamDefaultController, CountQueuingStrategy, ByteLengthQueuingStrategy,
};

Reflect.deleteProperty(WritableStreamDefaultController.prototype, "abortReason");
const streamLengths = new WeakMap<ReadableStream, number>();

// Record that `stream` delivers exactly `length` bytes.
export function setStreamLength<S extends ReadableStream>(stream: S, length: number): S {
  streamLengths.set(stream, length);
  return stream;
}

// The number of bytes `stream` delivers, when known.
export function streamLength(stream: ReadableStream): number | undefined {
  return streamLengths.get(stream);
}

const encoder = new TextEncoder();

// The bytes of a chunk a byte stream's writable side takes: a string or an [AllowShared] BufferSource.
export function writtenChunkBytes(chunk: unknown): Uint8Array {
  const bytes = typeof chunk === "string" ? encoder.encode(chunk) : sharedBufferSourceBytes(chunk);
  if (bytes) return bytes;
  throw new TypeError("This TransformStream is being used as a byte stream, but received an object of non-ArrayBuffer/ArrayBufferView type on its writable side.");
}

// The bytes of a chunk read from a stream to drain it: a string or a BufferSource.
export function drainedChunkBytes(chunk: unknown): Uint8Array {
  const bytes = typeof chunk === "string" ? encoder.encode(chunk) : bufferSourceBytes(chunk);
  if (bytes) return bytes;
  throw new TypeError("Draining read encountered a value that cannot be converted to bytes");
}

// Both branches of a tee deliver the bytes of the stream they split.
const teeBranches = ReadableStream.prototype.tee;
Object.defineProperty(ReadableStream.prototype, "tee", {
  ...Object.getOwnPropertyDescriptor(ReadableStream.prototype, "tee"),
  value: {
    tee(this: ReadableStream) {
      const branches = teeBranches.call(this);
      const length = streamLengths.get(this);
      if (length !== undefined) for (const branch of branches) streamLengths.set(branch, length);
      return branches;
    },
  }.tee,
});

export function nativeStreamError(reason: unknown): unknown {
  if (reason !== null && typeof reason === "object") {
    try { return structuredClone(reason); }
    catch { return new Error("#<Object>"); }
  }
  return new Error(reason === undefined ? "Stream was cancelled." : String(reason));
}

export function nativeReadableStream(source: UnderlyingByteSource): ReadableStream<Uint8Array> {
  const stream = new ReadableStream(source);
  setNativeCancelError(stream, nativeStreamError);
  return stream;
}

// The readable and writable sides of a transform stream tokamak implements.
class NativePair {
  declare readonly readable: ReadableStream;
  declare readonly writable: WritableStream;
  constructor(readable: ReadableStream, writable: WritableStream) {
    this.readable = readable;
    this.writable = writable;
  }
}

export class TransformStream {
  #pair: { readonly readable: ReadableStream; readonly writable: WritableStream };
  constructor(transformer?: Transformer | NativePair, writableStrategy?: QueuingStrategy, readableStrategy?: QueuingStrategy) {
    markHostObject(this);
    this.#pair = transformer instanceof NativePair ? transformer : new StandardTransformStream(transformer, writableStrategy, readableStrategy);
  }
  get readable() { return this.#pair.readable; }
  get writable() { return this.#pair.writable; }
}
for (const name of ["readable", "writable"]) {
  Object.defineProperty(TransformStream.prototype, name, { ...Object.getOwnPropertyDescriptor(TransformStream.prototype, name), enumerable: true });
}

export function isDisturbed(stream: ReadableStream | null | undefined): boolean { return Boolean(stream?._disturbed); }

Object.defineProperty(ReadableStreamBYOBReader.prototype, "readAtLeast", {
  configurable: true, enumerable: true, writable: true,
  value(this: ReadableStreamBYOBReader, minimumBytes: unknown, view: unknown) {
    const minimum = Math.trunc(Number(minimumBytes));
    if (!Number.isFinite(minimum) || minimum < 1) return Promise.reject(new TypeError("Minimum byte count must be positive"));
    if (!ArrayBuffer.isView(view) || minimum > view.byteLength) return Promise.reject(new TypeError("BYOB read requires a view large enough for the minimum"));
    const elementSize = "BYTES_PER_ELEMENT" in view && typeof view.BYTES_PER_ELEMENT === "number" ? view.BYTES_PER_ELEMENT : 1;
    return this.read(view, { min: Math.ceil(minimum / elementSize) });
  },
});

Object.defineProperty(ReadableStreamBYOBRequest.prototype, "atLeast", {
  configurable: true, enumerable: true,
  get(this: ReadableStreamBYOBRequest) {
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
  #decoder: TextDecoder;
  constructor(label: unknown = "utf-8", options: unknown = {}) {
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

function compressionPair(format: unknown, decode: boolean): NativePair {
  let codec: CompressionCodec | null = createCompression(String(format), decode);
  let readableController: ReadableByteStreamController;
  let writableController: WritableStreamDefaultController;
  const readable = nativeReadableStream({
    type: "bytes",
    start(controller) { readableController = controller; },
    cancel(reason) { codec = null; writableController.error(reason); },
  });
  const process = (chunk: unknown, finish: boolean) => {
    try {
      const bytes = typeof chunk === "string" ? encoder.encode(chunk) : sharedBufferSourceBytes(chunk);
      if (!bytes) throw new TypeError("This WritableStream only supports writing byte types.");
      if (codec === null) throw new TypeError("The compression stream has ended.");
      codec.write(bytes, finish, output => readableController.enqueue(output));
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
  return new NativePair(readable, writable);
}

export class CompressionStream extends TransformStream {
  constructor(format: unknown) { super(compressionPair(format, false)); }
}

export class DecompressionStream extends TransformStream {
  constructor(format: unknown) { super(compressionPair(format, true)); }
}

// A write the readable side has not yet delivered in full.
interface PendingWrite {
  bytes: Uint8Array;
  offset: number;
  length: number;
  completion: PromiseWithResolvers<void>;
}

function identityPair(remainingBytes: bigint | undefined, strategy: unknown = {}): NativePair {
  let remaining = remainingBytes;
  if (strategy === null) strategy = {};
  if (!isObject(strategy) || typeof strategy === "function") throw new TypeError("Expected a queuing strategy");
  let readableController: ReadableByteStreamController;
  let writableController: WritableStreamDefaultController;
  let available = Promise.withResolvers<void>();
  let pending: PendingWrite | undefined;
  let ended = false;
  const fail = (reason: unknown) => {
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
      if (ended || pending === undefined) return;
      const { bytes, completion } = pending;
      const request = readableController.byobRequest;
      if (request) {
        const view = request.view;
        if (view === null) return;
        const count = Math.min(view.byteLength, bytes.byteLength - pending.offset);
        view.set(bytes.subarray(pending.offset, pending.offset + count));
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
        const completion = Promise.withResolvers<void>();
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
    highWaterMark: Number(strategy.highWaterMark),
    size: (chunk: unknown) => typeof chunk === "string" ? chunk.length * 3 : isObject(chunk) && chunk.byteLength !== undefined ? Number(chunk.byteLength) : 1,
  });
  return new NativePair(readable, writable);
}

export class IdentityTransformStream extends TransformStream {
  constructor(strategy?: unknown, pair?: NativePair) {
    super(pair instanceof NativePair ? pair : identityPair(undefined, strategy));
  }
}

export class FixedLengthStream extends IdentityTransformStream {
  constructor(length: unknown, strategy?: unknown) {
    let remaining: bigint;
    try { remaining = typeof length === "bigint" ? length : BigInt(Math.trunc(Number(length))); }
    catch { throw new TypeError("Expected a non-negative length fitting in uint64"); }
    if (remaining < 0n || remaining > 0xffffffffffffffffn) throw new TypeError("Expected a non-negative length fitting in uint64");
    super(undefined, identityPair(remaining, strategy));
    setStreamLength(this.readable, Number(remaining));
  }
}

for (const Type of [TransformStream, CompressionStream, DecompressionStream, IdentityTransformStream, FixedLengthStream]) {
  Object.defineProperty(Type.prototype, Symbol.toStringTag, { value: Type.name, configurable: true });
}
