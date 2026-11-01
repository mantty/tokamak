// The vendored readable-stream package, which implements node:stream.
import * as stream from "node:stream";

export { PassThrough, finished, promises } from "node:stream";

/** A callback of a stream implementation, which readable-stream calls with any value as the error. */
export type StreamCallback = (error?: unknown, data?: unknown) => void;

// The members a stream implementation provides.
type Implementation = "_read" | "_write" | "_final" | "_destroy" | "_transform" | "_flush" | "destroy";

/** A `Transform` as readable-stream implements it, taking any value as an error. */
export interface Transform extends Omit<stream.Transform, Implementation> {
  _read(size: number): void;
  _write(chunk: unknown, encoding: BufferEncoding, callback: StreamCallback): void;
  _final(callback: StreamCallback): void;
  _destroy(error: unknown, callback: StreamCallback): void;
  _transform(chunk: unknown, encoding: BufferEncoding, callback: StreamCallback): void;
  _flush(callback: StreamCallback): void;
  destroy(error?: unknown): this;
}
// readable-stream reads the options it knows from any value.
export const Transform: { new (options?: unknown): Transform; readonly prototype: Transform };

/** A `Duplex`, which a constructor written before ES classes can also call. */
export const Duplex: typeof stream.Duplex & ((this: stream.Duplex, options?: unknown) => void);
export type Duplex = stream.Duplex;

// Node's internal state classes, which readable-stream exposes.
type StreamState = new (options: object | undefined, stream: stream.Stream, isDuplex?: boolean) => object;

export class Readable extends stream.Readable {
  static ReadableState: StreamState;
  /** Takes `n` bytes or objects from the buffered `state`. */
  static _fromList(n: number, state: object): unknown;
  /** Wraps a stream written in the style before Node 0.10 as a `Readable`. */
  static wrap(source: NodeJS.ReadableStream, options?: stream.ReadableOptions): Readable;
  /** Destroys the stream, taking any value as its error. */
  destroy(error?: unknown): this;
}

export class Writable extends stream.Writable {
  static WritableState: StreamState;
}

/** Wraps a stream written in the style before Node 0.10 as a `Duplex`. */
export class Wrapper extends stream.Duplex {}
