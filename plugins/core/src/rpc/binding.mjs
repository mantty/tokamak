import { RpcTarget, WorkerEntrypoint } from 'cloudflare:workers';
import { invoke } from './client.mjs';
import { serveRpc } from './endpoint.mjs';

const original = Symbol.for('tokamak.dev.service');

class Port extends RpcTarget {
  #socket;

  constructor(socket) { super(); this.#socket = socket; }
  receive(frame) { this.#socket.dispatchEvent(new MessageEvent('message', { data: frame })); }
  [Symbol.dispose]() { this.#socket.close(); }
}

/** Workerd owns these ports for the lifetime of the calling invocation. */
class Socket extends EventTarget {
  #peer;
  readyState = 0;

  connect(peer) {
    if (this.readyState === 3) { peer[Symbol.dispose](); return; }
    this.#peer = peer;
    this.readyState = 1;
    this.dispatchEvent(new Event('open'));
  }
  fail(error) { this.dispatchEvent(new ErrorEvent('error', { error })); this.close(); }
  send(frame) { return this.#peer.receive(frame); }
  close() {
    if (this.readyState === 3) return;
    this.readyState = 3;
    this.#peer?.[Symbol.dispose]();
    this.dispatchEvent(new Event('close'));
  }
}

/** Open the receiving invocation using workerd's internal RPC ownership. */
export function createRpcPort(peer, createWorker, methods, ctx) {
  const socket = new Socket();
  socket.connect(peer.dup());
  ctx.waitUntil(serveRpc(socket, createWorker, methods, ctx));
  return new Port(socket);
}

/** Enforce Tokamak's RPC contract over a development worker's service binding. */
export function createBinding(service, methods) {
  const binding = {
    ...Object.fromEntries(methods.map(method => [method, async (...args) => {
      const socket = new Socket();
      service.__tokamakRpc(new Port(socket)).then(peer => socket.connect(peer), error => socket.fail(error));
      return invoke(socket, method, args);
    }])),
    async fetch(input, init) {
      const response = await service.fetch(input, init);
      if (response.status === 101) {
        response.webSocket?.accept();
        response.webSocket?.close(1000);
        throw new TypeError('Service binding WebSocket upgrades are not supported');
      }
      return response;
    },
  };
  Object.defineProperty(binding, original, { value: service });
  return binding;
}

/** Install generated local bindings before the application's modules are evaluated. */
export function installBindings(env, bindings) {
  for (const [name, methods] of Object.entries(bindings)) {
    const binding = env[name];
    const service = Object.getOwnPropertyDescriptor(binding, original)?.value ?? binding;
    env[name] = createBinding(service, methods);
  }
  return env;
}

function instance(Type, ctx, env) {
  return typeof Type === 'function' ? new Type(ctx, env) : Type;
}

/** Route development RPC through the shared contract and preserve standard Worker handlers. */
export function developmentEntrypoint(Type, methods, bindings) {
  class Entry extends WorkerEntrypoint {
    __tokamakRpc(peer) {
      const env = installBindings(this.env, bindings);
      return createRpcPort(peer, ctx => instance(Type, ctx, env), methods, this.ctx);
    }
  }
  for (const name of ['fetch', 'queue', 'scheduled', 'email', 'tail', 'trace', 'tailStream', 'test']) {
    if (typeof (typeof Type === 'function' ? Type.prototype : Type)?.[name] !== 'function') continue;
    Object.defineProperty(Entry.prototype, name, {
      value(event) {
        const env = installBindings(this.env, bindings);
        const worker = instance(Type, this.ctx, env);
        return worker[name](event, env, this.ctx);
      },
    });
  }
  return Entry;
}

/** Install local bindings for host event dispatch. */
export function developmentEvents(Type, bindings) {
  return class extends Type {
    dispatch(name, data) {
      installBindings(this.env, bindings);
      return super.dispatch(name, data);
    }
  };
}
