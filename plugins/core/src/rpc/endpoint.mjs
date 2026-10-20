import { openChannel } from './channel.mjs';
import { dataCloneError } from './values.mjs';

/** Upgrade a same-origin frontend request into one RPC invocation. */
export function rpcResponse(request, createWorker, methods, ctx) {
  if (request.method !== 'GET') return new Response('Method not allowed', { status: 405 });
  const origin = request.headers.get('origin');
  if (origin && origin !== new URL(request.url).origin) return new Response('Forbidden', { status: 403 });
  if (request.headers.get('upgrade')?.toLowerCase() !== 'websocket') {
    return new Response('WebSocket required', { status: 426 });
  }
  const [client, server] = Object.values(new WebSocketPair());
  server.accept();
  ctx.waitUntil(serveRpc(server, createWorker, methods, ctx));
  return new Response(null, { status: 101, webSocket: client });
}

/** Serve one invocation, retaining it until the connection's streams settle. */
export function serveRpc(socket, createWorker, methods, ctx) {
  const channel = openChannel(socket);
  const waitUntil = promise => {
    ctx.waitUntil(promise);
    channel.waitUntil(promise);
  };
  const context = new Proxy(ctx, {
    get(target, key) {
      if (key === 'waitUntil') return waitUntil;
      const value = Reflect.get(target, key, target);
      return typeof value === 'function' ? value.bind(target) : value;
    },
  });
  async function handle() {
    try {
      const request = await channel.receive;
      if (!request || typeof request.method !== 'string' || !Array.isArray(request.args)) throw dataCloneError();
      if (!methods.includes(request.method)) throw new TypeError(`Unknown RPC method: ${request.method}`);
      const worker = createWorker(context);
      const value = await worker[request.method](...request.args);
      await channel.send({ failed: false, value });
    } catch (error) {
      try { await channel.send({ failed: true, value: error }); }
      catch { await channel.send({ failed: true, value: dataCloneError() }); }
    } finally { channel.complete(); }
  }
  handle().catch(channel.close);
  return channel.closed;
}
