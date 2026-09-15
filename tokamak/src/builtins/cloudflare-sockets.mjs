import { socketClose, socketConnect, socketRead, socketStartTls, socketWrite } from "tokamak:host";
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

function createState(address, mode, handle, openError, shared = undefined) {
  const closed = shared?.closed ?? deferred();
  const opened = deferred();
  const state = {
    address,
    mode,
    secureTransport: mode,
    handle,
    openError,
    opened,
    closed,
    upgraded: false,
    transferred: false,
    readable: null,
    writable: null,
  };
  if (openError === undefined) state.opened.resolve({ remoteAddress: socketLabel(address) });
  else {
    state.opened.reject(openError);
    state.closed.reject(openError);
  }
  const readable = new ReadableStream({
    type: "bytes",
    pull(controller) {
      if (state.transferred) {
        controller.error(new TypeError("Socket was transferred"));
        return;
      }
      if (state.handle === 0) {
        if (state.openError !== undefined) controller.error(state.openError);
        else controller.close();
        return;
      }
      try {
        const chunk = socketRead(state.handle);
        if (chunk == null) {
          closeState(state);
          controller.close();
        } else controller.enqueue(new Uint8Array(chunk));
      } catch (error) {
        closeState(state, error);
        controller.error(error);
      }
    },
    cancel(reason) { closeState(state, reason); },
  });
  const writable = new WritableStream({
    write(value) {
      if (state.transferred) throw new TypeError("Socket was transferred");
      if (state.handle === 0) throw state.openError ?? new TypeError("Socket is closed");
      socketWrite(state.handle, bytes(value));
    },
    close() { closeState(state); },
    abort(reason) { closeState(state, reason); },
  });
  state.readable = readable;
  state.writable = writable;
  return state;
}

function closeState(state, reason) {
  if (state.handle !== 0) {
    socketClose(state.handle);
    state.handle = 0;
  }
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
  const handle = state.handle;
  if (handle !== 0) socketStartTls(handle, state.address.hostname);
  state.handle = 0;
  const upgraded = createState(state.address, "on", handle, state.openError, state);
  return new Socket(upgraded);
}

export function connect(address, options = {}) {
  if (options === null || typeof options !== "object") throw new TypeError("Socket options must be an object");
  const target = validateAddress(socketAddress(address));
  const mode = secureMode(options);
  let handle = 0;
  let openError;
  try {
    handle = socketConnect(target.hostname, target.port, mode === "on");
  } catch (error) {
    openError = error;
  }
  return new Socket(createState(target, mode, handle, openError));
}

export function internalNewHttpClient() {
  return { fetch: globalThis.fetch };
}
