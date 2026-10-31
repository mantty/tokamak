import type { JsMessage, ServiceCall } from 'tokamak:host';
import { encode } from '../rpc/values.ts';
import { encodeRequest, decodeResponse, responseParts } from '../rpc/http.ts';
import { replyValue } from './streams.ts';

/** Calls `method` on the service bound as `name` with the arguments `message`. */
export type ServiceCaller = (name: string, method: string, message: JsMessage) => ServiceCall;

export async function fetchService(call: ServiceCaller, name: string, input: RequestInfo | URL, init?: RequestInit): Promise<Response> {
  const request = new Request(input, init);
  const { signal } = request;
  signal.throwIfAborted();
  const operation = call(name, 'fetch', encode(encodeRequest(request)));
  const abort = () => { operation.cancellation.cancel(); };
  signal.addEventListener('abort', abort, { once: true });
  const finish = () => { signal.removeEventListener('abort', abort); };
  let parts;
  try {
    const reply = await operation.reply;
    signal.throwIfAborted();
    parts = responseParts(replyValue(reply));
  } catch (error) {
    finish();
    signal.throwIfAborted();
    throw error;
  }
  if (parts.body === null) {
    finish();
    return decodeResponse(parts);
  }
  const reader = parts.body.getReader();
  parts.body = new ReadableStream({
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
      finally { operation.cancellation.cancel(); finish(); }
    },
  });
  return decodeResponse(parts);
}
