import { dataCloneError, field } from './values.ts';

const chunkSize = 64 * 1024;

/** A byte stream's next chunk, or its end. */
export type ByteRead = { done: true } | { done: false; value: Uint8Array };

/** A byte stream locked by the transport. */
export interface StreamSource {
  read(): Promise<ByteRead>;
  cancel(reason: unknown): Promise<void>;
}

/** The transport's side of a peer's byte stream: `read` resolves to the peer's read result. */
export interface StreamPort {
  read(): Promise<unknown>;
  cancel(reason: unknown): Promise<unknown>;
}

/** Lock a byte stream for the transport, reading only in response to receiver demand. */
export function takeStream(stream: ReadableStream<unknown>): StreamSource {
  let reader: ReadableStreamBYOBReader;
  try { reader = stream.getReader({ mode: 'byob' }); }
  catch { return chunks(stream.getReader()); }
  return {
    async read() {
      const next = await reader.read(new Uint8Array(chunkSize));
      return next.done ? { done: true } : { done: false, value: next.value };
    },
    cancel: reason => reader.cancel(reason),
  };
}

/** A source of a stream without byte reads, whose chunks must be unshared `Uint8Array`s, in pieces of at most `chunkSize`. */
function chunks(reader: ReadableStreamDefaultReader<unknown>): StreamSource {
  let remaining: Uint8Array = new Uint8Array(0);
  return {
    async read() {
      while (remaining.byteLength === 0) {
        const next = await reader.read();
        if (next.done) return { done: true };
        if (!(next.value instanceof Uint8Array) || (typeof SharedArrayBuffer !== 'undefined' && next.value.buffer instanceof SharedArrayBuffer)) {
          const error = dataCloneError();
          // Cleanup cannot delay or replace the invalid chunk's failure.
          reader.cancel(error).catch(() => {});
          throw error;
        }
        remaining = next.value;
      }
      const value = remaining.slice(0, chunkSize);
      remaining = remaining.subarray(value.byteLength);
      return { done: false, value };
    },
    cancel: reason => { remaining = new Uint8Array(0); return reader.cancel(reason); },
  };
}

/** A byte stream backed by a transport port's read and cancel operations. */
export function receiveStream(port: StreamPort): ReadableStream<Uint8Array> {
  return new ReadableStream({
    type: 'bytes',
    async pull(controller) {
      const result = await port.read();
      if (field(result, 'done')) {
        controller.close();
        controller.byobRequest?.respond(0);
        return;
      }
      const value = field(result, 'value');
      if (!(value instanceof Uint8Array) || value.byteLength === 0 || value.byteLength > chunkSize) {
        const error = dataCloneError();
        Promise.resolve().then(() => port.cancel(error)).catch(() => {});
        throw error;
      }
      controller.enqueue(value);
    },
    async cancel(reason) { await port.cancel(reason); },
  });
}
