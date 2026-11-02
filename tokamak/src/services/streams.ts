import type { JsMessage, JsReply, ServiceStreamPort, StreamCodec, StreamSource } from 'tokamak:host';
import { encode, decode, dataCloneError } from '../rpc/values.ts';
import { takeStream, receiveStream } from '../rpc/streams.ts';

export function failure(error: unknown): JsReply {
  try { return { failed: true, value: encode(error) }; }
  catch { return { failed: true, value: encode(dataCloneError()) }; }
}

export function openServiceStream(stream: ReadableStream): StreamSource {
  const source = takeStream(stream);
  return {
    async read() {
      try {
        const result = await source.read();
        return { done: result.done, failed: false, value: encode(result) };
      } catch (error) { return { done: true, ...failure(error) }; }
    },
    async cancel(reason: JsMessage) {
      try { await source.cancel(decode(reason)); return { failed: false, value: encode(undefined) }; }
      catch (error) { return failure(error); }
    },
  };
}

export function replyValue(reply: JsReply): unknown {
  const value = decode(reply.value);
  if (reply.failed) throw value;
  return value;
}

export function receiveServiceStream(port: ServiceStreamPort): ReadableStream<Uint8Array> {
  return receiveStream({
    read: async () => replyValue(await port.read(serviceStreamCodec)),
    cancel: async reason => replyValue(await port.cancel(encode(reason), serviceStreamCodec)),
  });
}

export const serviceStreamCodec: StreamCodec = { openServiceStream, receiveServiceStream };
