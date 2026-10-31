import { Channel, type RpcSocket } from './channel.ts';
import { dataCloneError, field, list } from './values.ts';

/** Create the application's worker for one invocation, given its execution context. */
export type CreateWorker = (ctx: ExecutionContext) => unknown;

/** Call `target[name](...args)`, as a property call does. */
export function callMethod(target: unknown, name: string, args: readonly unknown[]): unknown {
  const method: unknown = typeof target === 'object' && target !== null ? Reflect.get(target, name) : undefined;
  if (typeof method !== 'function') throw new TypeError(`${name} is not a function`);
  return Reflect.apply(method, target, args);
}

/** Upgrade a same-origin frontend request into one RPC invocation. */
export function rpcResponse(request: Request, createWorker: CreateWorker, methods: readonly string[], ctx: ExecutionContext): Response {
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
export function serveRpc(socket: RpcSocket, createWorker: CreateWorker, methods: readonly string[], ctx: ExecutionContext): Promise<unknown> {
  const channel = new Channel(socket);
  const waitUntil = (promise: Promise<unknown>) => {
    ctx.waitUntil(promise);
    channel.waitUntil(promise);
  };
  const context = new Proxy(ctx, {
    get(target, key) {
      if (key === 'waitUntil') return waitUntil;
      const value: unknown = Reflect.get(target, key, target);
      if (typeof value !== 'function') return value;
      const bound: unknown = value.bind(target);
      return bound;
    },
  });
  async function handle() {
    try {
      const request = await channel.receive;
      const method = field(request, 'method'), args = list(field(request, 'args'));
      if (typeof method !== 'string') throw dataCloneError();
      if (!methods.includes(method)) throw new TypeError(`Unknown RPC method: ${method}`);
      const value = await callMethod(createWorker(context), method, args);
      await channel.send({ failed: false, value });
    } catch (error) {
      try { await channel.send({ failed: true, value: error }); }
      catch { await channel.send({ failed: true, value: dataCloneError() }); }
    } finally { channel.complete(); }
  }
  handle().catch(channel.close);
  return channel.closed;
}
