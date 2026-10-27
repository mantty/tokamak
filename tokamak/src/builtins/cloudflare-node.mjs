import { codedError } from "../node/errors.mjs";
import { tcpPorts } from "../node/tcp-ports.mjs";

function validatePort(port) {
  if (!Number.isFinite(port) || port < 0 || port > 65535) throw new Error("Failed to determine port for server");
  return port;
}

function invalidArgument(message) {
  return codedError(Error, message, "ERR_INVALID_ARG_VALUE");
}

function serverPort(server) {
  const port = server.address()?.port;
  return typeof port === "number" ? port : null;
}

// The port `descriptor` names: a number, { port }, or a Node-style server,
// which listen() starts when it has no address.
function resolvePort(descriptor) {
  if (typeof descriptor === "number") descriptor = { port: descriptor };
  if (descriptor == null) throw new Error("Server descriptor cannot be null or undefined");
  if (descriptor.port == null && typeof descriptor.listen === "function") {
    const port = serverPort(descriptor);
    if (port !== null) return validatePort(port);
    // listen() assigns the port synchronously.
    descriptor.listen();
    return validatePort(serverPort(descriptor));
  }
  return validatePort(typeof descriptor.port === "number" ? descriptor.port : null);
}

// The port of an inbound socket's declared local address.
function portFromAuthority(authority) {
  const match = typeof authority === "string" ? /:(\d+)$/.exec(authority) : null;
  if (match === null) throw new Error(`Failed to determine port for inbound socket from local address ${String(authority)}`);
  return validatePort(Number(match[1]));
}

export async function handleAsNodeRequest(desc, request, env, ctx) {
  if (typeof desc === "number") desc = { port: desc };
  const port = validatePort(desc?.port);
  const instance = tcpPorts.getHandler(port);
  if (!instance || !("fetch" in instance)) {
    throw invalidArgument(`Http server with port ${port} not found. This is likely a bug with your code. `
      + `You should check if server.listen() was called with the same port (${port})`);
  }
  return await instance.fetch(request, env, ctx);
}

// Routes an inbound socket to the net.Server listening on the port it arrived
// on; resolves when the connection is finished.
export async function handleAsNodeConnection(socket, env, ctx) {
  const port = portFromAuthority((await socket.opened).localAddress);
  const instance = tcpPorts.getHandler(port);
  if (!instance || !("connect" in instance)) {
    throw invalidArgument(`No net.Server is listening on port ${port}. Call server.listen(${port}) to accept `
      + "connections arriving on that port.");
  }
  await instance.connect(socket, env, ctx);
}

export function connectHandler() {
  return {
    async connect(socket, env, ctx) {
      await handleAsNodeConnection(socket, env, ctx);
    },
  };
}

export function httpServerHandler(desc) {
  const port = resolvePort(desc);
  return {
    async fetch(request, env, ctx) {
      return await handleAsNodeRequest({ port }, request, env, ctx);
    },
  };
}
