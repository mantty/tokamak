import { encode } from '../rpc/values.mjs';
import { encodeRequest, decodeResponse } from '../rpc/http.mjs';
import { replyValue } from './streams.mjs';

export async function fetchService(call, name, input, init) {
  const request = new Request(input, init);
  const { signal } = request;
  signal.throwIfAborted();
  const operation = call(name, 'fetch', encode(encodeRequest(request)));
  const abort = () => operation.cancel();
  signal.addEventListener('abort', abort, { once: true });
  const finish = () => signal.removeEventListener('abort', abort);
  let value;
  try {
    const reply = await operation.result;
    signal.throwIfAborted();
    value = replyValue(reply);
  } catch (error) {
    finish();
    signal.throwIfAborted();
    throw error;
  }
  if (value.body === null) {
    finish();
    return decodeResponse(value);
  }
  const reader = value.body.getReader();
  value.body = new ReadableStream({
    type: 'bytes',
    async pull(controller) {
      try {
        signal.throwIfAborted();
        const { done, value } = await reader.read();
        signal.throwIfAborted();
        if (done) { finish(); controller.close(); controller.byobRequest?.respond(0); }
        else controller.enqueue(value);
      } catch (error) { finish(); controller.error(signal.aborted ? signal.reason : error); }
    },
    async cancel(reason) {
      try { await reader.cancel(reason); }
      finally { operation.cancel(); finish(); }
    },
  });
  return decodeResponse(value);
}
