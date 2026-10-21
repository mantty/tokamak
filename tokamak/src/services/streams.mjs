import { encode, decode, dataCloneError } from '../rpc/values.mjs';
import { takeStream, receiveStream } from '../rpc/streams.mjs';

function failure(error) {
  try { return { failed: true, value: encode(error) }; }
  catch { return { failed: true, value: encode(dataCloneError()) }; }
}

export function openServiceStream(stream) {
  const source = takeStream(stream);
  return {
    async read() {
      try {
        const result = await source.read();
        return { done: result.done, failed: false, value: encode(result) };
      } catch (error) { return { done: true, ...failure(error) }; }
    },
    async cancel(reason) {
      try { await source.cancel(decode(reason)); return { failed: false, value: encode(undefined) }; }
      catch (error) { return failure(error); }
    },
  };
}

function result(reply) {
  const value = decode(reply.value);
  if (reply.failed) throw value;
  return value;
}

export function receiveServiceStream(read, cancel) {
  return receiveStream({
    read: async () => result(await read(serviceStreamCodec)),
    cancel: async reason => result(await cancel(encode(reason), serviceStreamCodec)),
  });
}

export const serviceStreamCodec = { openServiceStream, receiveServiceStream };
