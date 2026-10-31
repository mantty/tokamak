import { socketConnect, type SocketConnection } from "tokamak:host";
import { objectOf } from "../globals/objects.ts";
import { ReadableStream, WritableStream, writtenChunkBytes } from "../streams/web.ts";

interface SocketAddress {
  hostname: string;
  port: number;
}

type SecureTransport = "off" | "on" | "starttls";

interface SocketState {
  address: SocketAddress;
  mode: SecureTransport;
  native: SocketConnection;
  allowHalfOpen: boolean;
  terminated: boolean;
  // The reason the socket closed, when it closed with an error.
  error?: unknown;
  readEnded: boolean;
  writeEnded: boolean;
  opened: PromiseWithResolvers<{ remoteAddress: string }>;
  closed: PromiseWithResolvers<void>;
  transferred: boolean;
  readable: ReadableStream<Uint8Array>;
  writable: WritableStream;
}

function deferred<T>(): PromiseWithResolvers<T> {
  const value = Promise.withResolvers<T>();
  value.promise.catch(() => {});
  return value;
}

function socketAddress(address: unknown): SocketAddress {
  if (typeof address === "string") {
    const match = /^\[([^\]]+)\]:(\d+)$|^([^:]+):(\d+)$/.exec(address);
    if (!match) throw new TypeError("Specified address is missing port.");
    return { hostname: match[1] ?? match[3], port: Number(match[2] ?? match[4]) };
  }
  if (address === null || typeof address !== "object") throw new TypeError("Specified address is invalid.");
  const fields = objectOf(address);
  return { hostname: String(fields.hostname ?? ""), port: Number(fields.port) };
}

function validateAddress(address: SocketAddress): SocketAddress {
  if (!Number.isInteger(address.port) || address.port < 0 || address.port > 65535) {
    throw new TypeError(address.port < 0
      ? "The value cannot be converted because it is negative and this API expects a positive number."
      : address.port > 65535
        ? "Value out of range. Must be less than or equal to 65535."
        : "The value cannot be converted because it is not an integer.");
  }
  return address;
}

function secureMode(options: object): SecureTransport {
  const mode = objectOf(options).secureTransport ?? "off";
  if (mode !== "off" && mode !== "on" && mode !== "starttls") {
    throw new TypeError(`Unsupported value in secureTransport socket option: ${mode}`);
  }
  return mode;
}

// Closes a byte stream, ending a pending BYOB read.
function closeReadable(controller: ReadableByteStreamController): void {
  controller.close();
  controller.byobRequest?.respond(0);
}

class Socket {
  #state: SocketState;

  constructor(state: SocketState) {
    this.#state = state;
  }

  get opened() { return this.#state.opened.promise; }
  get closed() { return this.#state.closed.promise; }
  get readable() { return this.#state.readable; }
  get writable() { return this.#state.writable; }
  get secureTransport() { return this.#state.mode; }
  get upgraded() { return this.#state.transferred; }
  get startTls() { return () => startTls(this.#state); }

  close(): Promise<void> {
    closeState(this.#state);
    return Promise.resolve();
  }
}

function socketLabel(address: SocketAddress): string {
  return address.hostname.includes(":") && !address.hostname.startsWith("[")
    ? `[${address.hostname}]:${address.port}`
    : `${address.hostname}:${address.port}`;
}

function createState(
  address: SocketAddress,
  mode: SecureTransport,
  native: SocketConnection,
  allowHalfOpen: boolean,
  closed = deferred<void>(),
): SocketState {
  // The streams' algorithms first run after `state` is assigned below.
  const readable = new ReadableStream({
    type: "bytes",
    async pull(controller) {
      if (state.transferred) {
        controller.error(new TypeError("Socket was transferred"));
        return;
      }
      if (state.terminated) {
        if (state.error !== undefined) controller.error(state.error);
        else closeReadable(controller);
        return;
      }
      try {
        await state.opened.promise;
        const chunk = await native.read();
        if (chunk == null) {
          state.readEnded = true;
          if (!allowHalfOpen && !state.writeEnded) {
            await native.shutdown();
            state.writeEnded = true;
          }
          if (state.writeEnded) closeState(state);
          closeReadable(controller);
        } else controller.enqueue(chunk);
      } catch (error) {
        if ((state.terminated && state.error === undefined) || state.transferred) { closeReadable(controller); return; }
        closeState(state, error);
        controller.error(error);
      }
    },
    cancel(reason) { closeState(state, reason); },
  });
  const writable = new WritableStream({
    async write(value) {
      if (state.transferred) throw new TypeError("Socket was transferred");
      if (state.terminated) throw state.error ?? new TypeError("Socket is closed");
      const data = writtenChunkBytes(value);
      await state.opened.promise;
      await native.write(data);
    },
    async close() {
      await state.opened.promise;
      await native.shutdown();
      state.writeEnded = true;
      if (state.readEnded) closeState(state);
    },
    abort(reason) { closeState(state, reason); },
  });
  const state: SocketState = {
    address,
    mode,
    native,
    allowHalfOpen,
    terminated: false,
    readEnded: false,
    writeEnded: false,
    opened: deferred(),
    closed,
    transferred: false,
    readable,
    writable,
  };
  native.opened().then(() => state.opened.resolve({ remoteAddress: socketLabel(address) }), error => {
    state.opened.reject(error);
    closeState(state, error);
  });
  return state;
}

function closeState(state: SocketState, reason?: unknown): void {
  if (state.transferred || state.terminated) return;
  state.native.close();
  state.terminated = true;
  state.error = reason;
  if (reason === undefined) state.closed.resolve();
  else state.closed.reject(reason);
}

function startTls(state: SocketState): Socket {
  if (state.mode !== "starttls") {
    throw new TypeError(state.mode === "on"
      ? "Cannot startTls on a TLS socket."
      : "The `secureTransport` socket option must be set to 'starttls' for startTls to be used.");
  }
  if (state.transferred) throw new TypeError("startTls has already been called on this socket, or the socket was transferred over RPC.");
  state.transferred = true;
  const native = state.native.startTls(state.address.hostname);
  return new Socket(createState(state.address, "on", native, state.allowHalfOpen, state.closed));
}

export function connect(address: unknown, options: unknown = {}): Socket {
  if (options === null || typeof options !== "object") throw new TypeError("Socket options must be an object");
  const target = validateAddress(socketAddress(address));
  const mode = secureMode(options);
  const native = socketConnect(target.hostname, target.port, mode === "on");
  return new Socket(createState(target, mode, native, Boolean(objectOf(options).allowHalfOpen)));
}

// workerd provides the HTTP client only under its experimental compatibility flag.
export function internalNewHttpClient(socket: unknown): never {
  throw new TypeError("sockets.internalNewHttpClient is not a function");
}
