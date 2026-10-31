import { Channel, settlement, type RpcSocket, type Settlement } from './channel.ts';

/** Invoke one operation over a socket owned by the caller. */
export async function invoke(socket: RpcSocket, method: string, args: unknown[]): Promise<unknown> {
  const channel = new Channel(socket);
  let result: Settlement;
  try {
    await channel.send({ method, args });
    result = settlement(await channel.receive);
  } catch (error) {
    channel.close(error);
    throw error;
  }
  if (result.failed) throw result.value;
  return result.value;
}

/** The address of the page this module runs in, or undefined outside a page. */
function pageAddress(): string | undefined {
  const location: unknown = Reflect.get(globalThis, 'location');
  if (typeof location !== 'object' || location === null || !('href' in location)) return undefined;
  return typeof location.href === 'string' ? location.href : undefined;
}

/** Generate only the declared public operations, with their `capabilities`; each call owns its connection. */
export function createClient(methods: readonly string[], endpoint: string, capabilities: object) {
  const client = Object.fromEntries(methods.map(method => [method, async (...args: unknown[]) => {
    const url = new URL(endpoint, pageAddress());
    if (url.protocol === 'http:') url.protocol = 'ws:';
    if (url.protocol === 'https:') url.protocol = 'wss:';
    return invoke(new WebSocket(url.href), method, args);
  }]));
  return Object.assign(client, { capabilities: Object.freeze({ ...capabilities }) });
}
