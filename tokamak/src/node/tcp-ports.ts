import type { Response } from "../network/fetch.ts";
import { codedError } from "./errors.ts";

const EPHEMERAL_PORT_MIN = 49152;
const EPHEMERAL_PORT_MAX = 65535;

// Synthetic addresses from the never-routed 240.1.0.0/16: the isolate's own
// address, and the one address peers the platform does not identify appear behind.
export const HOST_ADDRESS = "240.1.0.1";
export const GATEWAY_ADDRESS = "240.1.255.254";

// A cloudflare:sockets socket, by a name that node:net's Socket does not shadow.
export type CloudflareSocket = Socket;

// Where a port's HTTP requests or socket connections go.
export type PortHandler =
  | { fetch(request: Request, env: unknown, ctx: ExecutionContext): Promise<Response> }
  | { connect(socket: CloudflareSocket, env: unknown, ctx: ExecutionContext): Promise<void> | void };

interface PortEntry {
  refs: number;
  reusePort: boolean;
  handlers: PortHandler[];
}

// The isolate's virtual local TCP ports. Bound ports are recorded so that they
// are unique within the isolate, and an entry carries the handlers that the
// port's requests and connections are routed to.
class PortTable {
  #entries = new Map<number, PortEntry>();
  #nextEphemeral = EPHEMERAL_PORT_MIN;
  // Releases the port of an owner collected without having released it.
  #registry = new FinalizationRegistry<number>(port => this.release(port));

  // The next ephemeral port that is not recorded, without recording it; 0 when
  // every one is.
  ephemeral(): number {
    for (let attempt = EPHEMERAL_PORT_MIN; attempt <= EPHEMERAL_PORT_MAX; attempt++) {
      const candidate = this.#nextEphemeral;
      this.#nextEphemeral = candidate === EPHEMERAL_PORT_MAX ? EPHEMERAL_PORT_MIN : candidate + 1;
      if (!this.#entries.has(candidate)) return candidate;
    }
    return 0;
  }

  // Records `port`, or shares it when its holder and this binder both set
  // `reusePort`; false on conflict.
  tryBind(port: number, reusePort = false): boolean {
    const entry = this.#entries.get(port);
    if (entry === undefined) this.#entries.set(port, { refs: 1, reusePort, handlers: [] });
    else if (entry.reusePort && reusePort) entry.refs++;
    else return false;
    return true;
  }

  release(port: number): void {
    const entry = this.#entries.get(port);
    if (entry !== undefined && --entry.refs === 0) this.#entries.delete(port);
  }

  // An owner unregisters before releasing, so that its port is released once.
  register(owner: object, port: number): void { this.#registry.register(owner, port, owner); }
  unregister(owner: object): void { this.#registry.unregister(owner); }

  setHandler(port: number, handler: PortHandler): void { this.#entries.get(port)?.handlers.push(handler); }

  clearHandler(port: number, handler: PortHandler): void {
    const handlers = this.#entries.get(port)?.handlers ?? [];
    const index = handlers.indexOf(handler);
    if (index !== -1) handlers.splice(index, 1);
  }

  // The port's handlers in turn.
  getHandler(port: number): PortHandler | undefined {
    const handlers = this.#entries.get(port)?.handlers ?? [];
    const handler = handlers.shift();
    if (handler !== undefined) handlers.push(handler);
    return handler;
  }
}

export const tcpPorts = new PortTable();

// The port `port` names, a number or a numeric string from 0 to 65535.
export function validatePort(port: unknown, name = "Port"): number {
  const numeric = typeof port === "number" || (typeof port === "string" && port.trim() !== "");
  if (!numeric || +port !== +port >>> 0 || +port > 0xffff) {
    throw codedError(Error, `${name} should be >= 0 and < 65536. Received ${typeof port}.`, "ERR_SOCKET_BAD_PORT");
  }
  return +port | 0;
}

// Records `port`, or an ephemeral port for 0, and returns it.
export function bindPort(address: string, port: number, reusePort = false): number {
  const bound = port === 0 ? tcpPorts.ephemeral() : port;
  if (bound === 0 || !tcpPorts.tryBind(bound, reusePort)) {
    throw Object.assign(codedError(Error, `bind EADDRINUSE ${address}:${bound}`, "EADDRINUSE"), { syscall: "bind", address, port: bound });
  }
  return bound;
}
