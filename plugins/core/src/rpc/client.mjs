import { openChannel } from './channel.mjs';
import { dataCloneError } from './values.mjs';

/** Invoke one operation over a socket owned by the caller. */
export async function invoke(socket, method, args) {
  const channel = openChannel(socket);
  let result;
  try {
    await channel.send({ method, args });
    result = await channel.receive;
    if (typeof result?.failed !== 'boolean') throw dataCloneError();
  } catch (error) {
    channel.close(error);
    throw error;
  }
  if (result.failed) throw result.value;
  return result.value;
}

/** Generate only the declared public operations; each call owns its connection. */
export function createClient(methods, endpoint) {
  return Object.fromEntries(methods.map(method => [method, async (...args) => {
    const url = new URL(endpoint, globalThis.location?.href);
    if (url.protocol === 'http:') url.protocol = 'ws:';
    if (url.protocol === 'https:') url.protocol = 'wss:';
    return invoke(new WebSocket(url), method, args);
  }]));
}
