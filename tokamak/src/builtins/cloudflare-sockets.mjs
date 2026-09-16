import { socketConnect } from "tokamak:host";
import { ReadableStream, WritableStream } from "../streams/web.mjs";

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  promise.catch(() => {});
  return { promise, resolve, reject };
}

function socketAddress(address) {
  if (typeof address === "string") {
    const match = /^\[([^\]]+)\]:(\d+)$|^([^:]+):(\d+)$/.exec(address);
    if (!match) throw new TypeError("Specified address is missing port.");
    return { hostname: match[1] ?? match[3], port: Number(match[2] ?? match[4]) };
  }
  if (address === null || typeof address !== "object") throw new TypeError("Specified address is invalid.");
  return { hostname: String(address.hostname ?? ""), port: Number(address.port) };
}

function validateAddress(address) {
  if (!Number.isInteger(address.port) || address.port < 0 || address.port > 65535) {
    throw new TypeError(address.port < 0
      ? "The value cannot be converted because it is negative and this API expects a positive number."
      : address.port > 65535
        ? "Value out of range. Must be less than or equal to 65535."
        : "The value cannot be converted because it is not an integer.");
  }
  return address;
}

function bytes(value) {
  if (value instanceof ArrayBuffer) return new Uint8Array(value);
  if (ArrayBuffer.isView(value)) return new Uint8Array(value.buffer, value.byteOffset, value.byteLength);
  throw new TypeError("Socket writes require an ArrayBuffer or ArrayBufferView");
}

function secureMode(options) {
  const mode = options?.secureTransport ?? "off";
  if (mode !== "off" && mode !== "on" && mode !== "starttls") {
    throw new TypeError(`Unsupported value in secureTransport socket option: ${mode}`);
  }
  return mode;
}

const socketStates = new WeakMap();

class Socket {
  constructor(state) {
    socketStates.set(this, state);
  }

  get opened() { return socketStates.get(this).opened.promise; }
  get closed() { return socketStates.get(this).closed.promise; }
  get readable() { return socketStates.get(this).readable; }
  get writable() { return socketStates.get(this).writable; }
  get secureTransport() { return socketStates.get(this).secureTransport; }
  get upgraded() { return socketStates.get(this).upgraded; }
  get startTls() { return () => startTls(this); }

  close() {
    const state = socketStates.get(this);
    closeState(state);
    return Promise.resolve();
  }
}

function socketLabel(address) {
  return address.hostname.includes(":") && !address.hostname.startsWith("[")
    ? `[${address.hostname}]:${address.port}`
    : `${address.hostname}:${address.port}`;
}

function createState(address, mode, native, allowHalfOpen, shared = undefined) {
  const closed = shared?.closed ?? deferred();
  const opened = deferred();
  const state = {
    address,
    mode,
    secureTransport: mode,
    native,
    allowHalfOpen,
    terminated: false,
    readEnded: false,
    writeEnded: false,
    opened,
    closed,
    upgraded: false,
    transferred: false,
    readable: null,
    writable: null,
  };
  native.opened.then(() => state.opened.resolve({ remoteAddress: socketLabel(address) }), error => {
    state.opened.reject(error);
    closeState(state, error);
  });
  const readable = new ReadableStream({
    type: "bytes",
    async pull(controller) {
      if (state.transferred) {
        controller.error(new TypeError("Socket was transferred"));
        return;
      }
      if (state.terminated) {
        if (state.error !== undefined) controller.error(state.error);
        else controller.close();
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
          controller.close();
        } else controller.enqueue(new Uint8Array(chunk));
      } catch (error) {
        if ((state.terminated && state.error === undefined) || state.transferred) { controller.close(); return; }
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
      const data = bytes(value);
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
  state.readable = readable;
  state.writable = writable;
  return state;
}

function closeState(state, reason) {
  if (state.transferred || state.terminated) return;
  state.native.close();
  state.terminated = true;
  state.error = reason;
  if (reason === undefined) state.closed.resolve();
  else state.closed.reject(reason);
}

function startTls(socket) {
  const state = socketStates.get(socket);
  if (state.mode !== "starttls") {
    throw new TypeError(state.mode === "on"
      ? "Cannot startTls on a TLS socket."
      : "The `secureTransport` socket option must be set to 'starttls' for startTls to be used.");
  }
  if (state.transferred) throw new TypeError("startTls has already been called on this socket, or the socket was transferred over RPC.");
  state.transferred = true;
  state.upgraded = true;
  const native = state.native.startTls(state.address.hostname);
  const upgraded = createState(state.address, "on", native, state.allowHalfOpen, state);
  return new Socket(upgraded);
}

export function connect(address, options = {}) {
  if (options === null || typeof options !== "object") throw new TypeError("Socket options must be an object");
  const target = validateAddress(socketAddress(address));
  const mode = secureMode(options);
  const native = socketConnect(target.hostname, target.port, mode === "on");
  return new Socket(createState(target, mode, native, Boolean(options.allowHalfOpen)));
}

export function internalNewHttpClient() {
  return { fetch: globalThis.fetch };
}
