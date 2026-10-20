import { RpcTarget } from 'cloudflare:workers';
import { invoke } from './client.mjs';
import { serveRpc } from './endpoint.mjs';

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
  return {
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
}
