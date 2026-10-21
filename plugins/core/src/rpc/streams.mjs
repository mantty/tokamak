import { dataCloneError } from './values.mjs';

const chunkSize = 64 * 1024;

/** Lock a byte stream for the transport, reading only in response to receiver demand. */
export function takeStream(stream) {
  let reader, byob = true, remaining = new Uint8Array(0);
  try { reader = stream.getReader({ mode: 'byob' }); }
  catch { byob = false; reader = stream.getReader(); }
  return {
    async read() {
      if (byob) return reader.read(new Uint8Array(chunkSize));
      while (remaining.byteLength === 0) {
        const next = await reader.read();
        if (next.done) return next;
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
export function receiveStream(port) {
  return new ReadableStream({
    type: 'bytes',
    async pull(controller) {
      const { done, value } = await port.read();
      if (done) {
        controller.close();
        controller.byobRequest?.respond(0);
        return;
      }
      if (!(value instanceof Uint8Array) || value.byteLength === 0 || value.byteLength > chunkSize) {
        const error = dataCloneError();
        Promise.resolve().then(() => port.cancel(error)).catch(() => {});
        throw error;
      }
      controller.enqueue(value);
    },
    cancel: reason => port.cancel(reason),
  });
}
