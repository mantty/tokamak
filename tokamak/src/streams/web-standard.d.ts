// The vendored web-streams-polyfill package, which implements the WHATWG
// Streams standard.
// Declared under other names, so that each type names the global class.
declare const byteLengthQueuingStrategy: typeof ByteLengthQueuingStrategy;
declare const countQueuingStrategy: typeof CountQueuingStrategy;
declare const readableByteStreamController: typeof ReadableByteStreamController;
declare const readableStream: typeof ReadableStream;
declare const readableStreamBYOBReader: typeof ReadableStreamBYOBReader;
declare const readableStreamBYOBRequest: typeof ReadableStreamBYOBRequest;
declare const readableStreamDefaultController: typeof ReadableStreamDefaultController;
declare const readableStreamDefaultReader: typeof ReadableStreamDefaultReader;
declare const transformStream: typeof TransformStream;
declare const transformStreamDefaultController: typeof TransformStreamDefaultController;
declare const writableStream: typeof WritableStream;
declare const writableStreamDefaultController: typeof WritableStreamDefaultController;
declare const writableStreamDefaultWriter: typeof WritableStreamDefaultWriter;
export {
  byteLengthQueuingStrategy as ByteLengthQueuingStrategy,
  countQueuingStrategy as CountQueuingStrategy,
  readableByteStreamController as ReadableByteStreamController,
  readableStream as ReadableStream,
  readableStreamBYOBReader as ReadableStreamBYOBReader,
  readableStreamBYOBRequest as ReadableStreamBYOBRequest,
  readableStreamDefaultController as ReadableStreamDefaultController,
  readableStreamDefaultReader as ReadableStreamDefaultReader,
  transformStream as TransformStream,
  transformStreamDefaultController as TransformStreamDefaultController,
  writableStream as WritableStream,
  writableStreamDefaultController as WritableStreamDefaultController,
  writableStreamDefaultWriter as WritableStreamDefaultWriter,
};

/** Makes `error` the reason a native cancel of `stream` reports. */
export function setNativeCancelError(stream: ReadableStream, error: unknown): void;

// The polyfill provides every stream global in the runtime, so its internal
// properties that tokamak reads are declared on the global types.
declare global {
  interface ReadableStream {
    readonly _disturbed?: boolean;
  }

  interface ReadableStreamBYOBReader {
    read<T extends ArrayBufferView>(view: T, options: { min: number }): Promise<ReadableStreamReadResult<T>>;
  }

  interface ReadableStreamBYOBRequest {
    readonly _associatedReadableByteStreamController?: {
      readonly _pendingPullIntos: { peek(): { readonly bytesFilled: number; readonly minimumFill: number } | undefined };
    };
  }
}
