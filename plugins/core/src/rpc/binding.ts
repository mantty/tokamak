import { RpcTarget, WorkerEntrypoint } from 'cloudflare:workers';
import type { eventEntrypoint } from '../entrypoint.ts';
import type { RpcSocket } from './channel.ts';
import { invoke } from './client.ts';
import { callMethod, serveRpc, type CreateWorker } from './endpoint.ts';

const original = Symbol.for('tokamak.dev.service');

/** A development worker's bindings. */
type Env = Record<string, unknown>;

/** The service bindings to install, each with its declared operations. */
type Bindings = Record<string, readonly string[]>;

/** The other invocation's `Port`, as a workerd RPC stub. */
interface PeerPort {
  receive(frame: ArrayBuffer): Promise<void>;
  dup(): PeerPort;
  [Symbol.dispose](): void;
}

/** A service binding to a development worker, whose entrypoints `developmentEntrypoint` made. */
interface DevelopmentService {
  __tokamakRpc(port: Port): Promise<PeerPort>;
  fetch(input: RequestInfo | URL, init?: RequestInit): Promise<Response>;
}

class Port extends RpcTarget {
  #socket: Socket;

  constructor(socket: Socket) { super(); this.#socket = socket; }
  receive(frame: ArrayBuffer): void { this.#socket.dispatchEvent(new MessageEvent('message', { data: frame })); }
  [Symbol.dispose](): void { this.#socket.close(); }
}

/** Workerd owns these ports for the lifetime of the calling invocation. */
class Socket extends EventTarget implements RpcSocket {
  #peer: PeerPort | undefined;
  readyState = 0;

  connect(peer: PeerPort): void {
    if (this.readyState === 3) { peer[Symbol.dispose](); return; }
    this.#peer = peer;
    this.readyState = 1;
    this.dispatchEvent(new Event('open'));
  }
  fail(error: unknown): void { this.dispatchEvent(new ErrorEvent('error', { error })); this.close(); }
  send(frame: ArrayBuffer): Promise<void> {
    if (!this.#peer) throw new DOMException('The RPC port is not connected.', 'InvalidStateError');
    return this.#peer.receive(frame);
  }
  close(): void {
    if (this.readyState === 3) return;
    this.readyState = 3;
    this.#peer?.[Symbol.dispose]();
    this.dispatchEvent(new Event('close'));
  }
}

/** Open the receiving invocation using workerd's internal RPC ownership. */
export function createRpcPort(peer: PeerPort, createWorker: CreateWorker, methods: readonly string[], ctx: ExecutionContext): Port {
  const socket = new Socket();
  socket.connect(peer.dup());
  ctx.waitUntil(serveRpc(socket, createWorker, methods, ctx));
  return new Port(socket);
}

/** Enforce Tokamak's RPC contract over a development worker's service binding. */
export function createBinding(service: DevelopmentService, methods: readonly string[]): object {
  const binding = {
    ...Object.fromEntries(methods.map(method => [method, async (...args: unknown[]) => {
      const socket = new Socket();
      service.__tokamakRpc(new Port(socket)).then(peer => { socket.connect(peer); }, (error: unknown) => { socket.fail(error); });
      return invoke(socket, method, args);
    }])),
    async fetch(input: RequestInfo | URL, init?: RequestInit): Promise<Response> {
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

/** The service binding `binding` wraps when `createBinding` made it, else `binding` itself. */
function serviceOf(binding: unknown): DevelopmentService {
  const wrapped: unknown = Object.getOwnPropertyDescriptor(binding, original)?.value;
  // tok lists only service bindings to development workers in `Bindings`, and installs them only here.
  return (wrapped ?? binding) as DevelopmentService;
}

/** Install generated local bindings before the application's modules are evaluated. */
export function installBindings(env: Env, bindings: Bindings): Env {
  for (const [name, methods] of Object.entries(bindings)) {
    env[name] = createBinding(serviceOf(env[name]), methods);
  }
  return env;
}

function instance(Type: unknown, ctx: ExecutionContext, env: Env): unknown {
  return typeof Type === 'function' ? Reflect.construct(Type, [ctx, env]) : Type;
}

const handlerNames = ['fetch', 'queue', 'scheduled', 'email', 'tail', 'trace', 'tailStream', 'test'];

/** Route development RPC through the shared contract and preserve standard Worker handlers. */
export function developmentEntrypoint(Type: unknown, methods: readonly string[], bindings: Bindings) {
  const handlers: unknown = typeof Type === 'function' ? Type.prototype : Type;
  class Entry extends WorkerEntrypoint<Env> {
    __tokamakRpc(peer: PeerPort): Port {
      const env = installBindings(this.env, bindings);
      return createRpcPort(peer, ctx => instance(Type, ctx, env), methods, this.ctx);
    }

    static {
      for (const name of handlerNames) {
        if (typeof handlers !== 'object' || handlers === null || typeof Reflect.get(handlers, name) !== 'function') continue;
        Object.defineProperty(this.prototype, name, {
          value(this: Entry, event: unknown) {
            const env = installBindings(this.env, bindings);
            return callMethod(instance(Type, this.ctx, env), name, [event, env, this.ctx]);
          },
        });
      }
    }
  }
  return Entry;
}

/** Install local bindings for host event dispatch. */
export function developmentEvents(Type: ReturnType<typeof eventEntrypoint>, bindings: Bindings) {
  return class extends Type {
    dispatch(name: string, data: Uint8Array) {
      installBindings(this.env, bindings);
      return super.dispatch(name, data);
    }
  };
}
