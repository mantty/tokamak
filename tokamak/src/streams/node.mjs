import EventEmitter from "../events/events.mjs";
import { TextDecoder } from "./text.mjs";

export class Writable extends EventEmitter {
  write(chunk, encoding, callback) { callback?.(); this.emit("drain"); return true; }
  end(chunk, encoding, callback) { if (chunk !== undefined) this.write(chunk, encoding); callback?.(); this.emit("finish"); return this; }
  destroy(error) { if (error) this.emit("error", error); this.emit("close"); return this; }
}

export class Readable extends EventEmitter {
  constructor(options = {}) {
    super();
    this.readable = true;
    this.__chunks = [];
    this.__read = options.read;
    this.__ended = false;
    this.__endEmitted = false;
    this.__decoder = options.encoding === undefined ? null : new TextDecoder(options.encoding);
  }
  setEncoding(encoding) {
    this.__decoder = new TextDecoder(encoding);
    return this;
  }
  on(name, listener) {
    super.on(name, listener);
    if (name === "data") {
      while (this.__chunks.length) this.emit("data", this.__chunks.shift());
      this.__emitEndIfReady();
    }
    if (name === "end" && this.__endEmitted) listener();
    return this;
  }
  push(chunk) {
    if (this.__ended) return false;
    if (chunk === null) {
      if (this.__decoder) {
        const value = this.__decoder.decode();
        if (value) this.__pushDecoded(value);
      }
      this.__ended = true;
      this.__emitEndIfReady();
    } else {
      const value = this.__decoder && !(typeof chunk === "string")
        ? this.__decoder.decode(chunk, { stream: true })
        : chunk;
      if (value) this.__pushDecoded(value);
    }
    return true;
  }
  __pushDecoded(value) {
    if (this.listenerCount("data")) this.emit("data", value);
    else this.__chunks.push(value);
  }
  __emitEndIfReady() {
    if (this.__ended && !this.__endEmitted && this.__chunks.length === 0) {
      this.__endEmitted = true;
      this.emit("end");
    }
  }
  read() { return this.__chunks.shift() ?? null; }
  pipe(destination) { this.on("data", (chunk) => destination.write(chunk)); this.once("end", () => destination.end()); return destination; }
  destroy(error) { if (error) this.emit("error", error); this.emit("close"); return this; }
}

export class Duplex extends Readable {
  write(chunk, encoding, callback) { callback?.(); this.emit("data", chunk); return true; }
  end(chunk, encoding, callback) { if (chunk !== undefined) this.write(chunk, encoding); callback?.(); this.emit("finish"); this.emit("end"); return this; }
}

export class Transform extends Duplex {}
export class PassThrough extends Transform {}
export class Stream extends Duplex {}
export class Wrapper extends Duplex {}
export class ReadableState {}
export class WritableState {}

export function destroy(stream, error) { return stream.destroy(error); }
export function isDestroyed(stream) { return Boolean(stream.__destroyed); }
export function isReadable(stream) { return stream?.readable !== false; }
export function isWritable(stream) { return stream?.writable !== false; }
export function addAbortSignal(signal, stream) { signal?.addEventListener("abort", () => stream.destroy(signal.reason), { once: true }); return stream; }
export async function pipeline(...streams) {
  const callback = typeof streams.at(-1) === "function" ? streams.pop() : null;
  try { for (let index = 0; index + 1 < streams.length; index += 1) streams[index].pipe(streams[index + 1]); callback?.(); return streams.at(-1); }
  catch (error) { callback?.(error); throw error; }
}
export function finished(stream, callback) { stream.once("end", callback); stream.once("error", callback); return () => {}; }
export const promises = { pipeline, finished };
export const consumers = {};
export function from(value) { return value instanceof Readable ? value : new Readable({ objectMode: true }); }
export function fromWeb(value) { return from(value); }
export function toWeb(value) { return value; }
export function _fromList() { return undefined; }
export function _isArrayBufferView(value) { return ArrayBuffer.isView(value); }
export function _isUint8Array(value) { return value instanceof Uint8Array; }
export function _uint8ArrayToBuffer(value) { return value; }
export function compose(...streams) { return pipeline(...streams); }
export function duplexPair() { return [new Duplex(), new Duplex()]; }
export function getDefaultHighWaterMark(objectMode) { return objectMode ? 16 : 16 * 1024; }
export function isDisturbed(stream) { return Boolean(stream?.__disturbed); }
export function isErrored(stream) { return Boolean(stream?.__errored); }
export function setDefaultHighWaterMark() {}
Object.assign(Stream, {
  Writable, Readable, Duplex, Transform, PassThrough, Stream, destroy, isReadable, isWritable, addAbortSignal, pipeline,
  finished, promises, _isArrayBufferView, _isUint8Array, _uint8ArrayToBuffer, compose, duplexPair,
  getDefaultHighWaterMark, isDestroyed, isDisturbed, isErrored, setDefaultHighWaterMark,
});
export default Stream;
