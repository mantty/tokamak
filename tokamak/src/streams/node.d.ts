// The vendored readable-stream package, which implements node:stream.
import * as stream from "node:stream";

export { Duplex, PassThrough, Transform, finished, promises } from "node:stream";

// Node's internal state classes, which readable-stream exposes.
type StreamState = new (options: object | undefined, stream: stream.Stream, isDuplex?: boolean) => object;

export class Readable extends stream.Readable {
  static ReadableState: StreamState;
  /** Takes `n` bytes or objects from the buffered `state`. */
  static _fromList(n: number, state: object): unknown;
  /** Wraps a stream written in the style before Node 0.10 as a `Readable`. */
  static wrap(source: NodeJS.ReadableStream, options?: stream.ReadableOptions): Readable;
}

export class Writable extends stream.Writable {
  static WritableState: StreamState;
}

/** Wraps a stream written in the style before Node 0.10 as a `Duplex`. */
export class Wrapper extends stream.Duplex {}
