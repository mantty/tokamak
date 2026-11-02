import type { Response } from "../network/fetch.ts";
import { codedError } from "../node/errors.ts";
import { tcpPorts } from "../node/tcp-ports.ts";

function validatePort(port: unknown): number {
  if (typeof port !== "number" || !Number.isFinite(port) || port < 0 || port > 65535) throw new Error("Failed to determine port for server");
  return port;
}

function invalidArgument(message: string) {
  return codedError(Error, message, "ERR_INVALID_ARG_VALUE");
}

function serverPort(server: Record<string, unknown>): number | null {
  if (typeof server.address !== "function") throw new TypeError("server.address is not a function");
  const address: unknown = server.address();
  const port: unknown = address == null ? undefined : Object(address).port;
  return typeof port === "number" ? port : null;
}

// The port `descriptor` names: a number, { port }, or a Node-style server,
// which listen() starts when it has no address.
function resolvePort(descriptor: unknown): number {
  if (typeof descriptor === "number") descriptor = { port: descriptor };
  if (descriptor == null) throw new Error("Server descriptor cannot be null or undefined");
  const server: Record<string, unknown> = Object(descriptor);
  if (server.port == null && typeof server.listen === "function") {
    const port = serverPort(server);
    if (port !== null) return validatePort(port);
    // listen() assigns the port synchronously.
    server.listen();
    return validatePort(serverPort(server));
  }
  return validatePort(typeof server.port === "number" ? server.port : null);
}

// The port of an inbound socket's declared local address.
function portFromAuthority(authority: unknown): number {
  const match = typeof authority === "string" ? /:(\d+)$/.exec(authority) : null;
  if (match === null) throw new Error(`Failed to determine port for inbound socket from local address ${String(authority)}`);
  return validatePort(Number(match[1]));
}

export async function handleAsNodeRequest(desc: unknown, request: Request, env: unknown, ctx: ExecutionContext): Promise<Response> {
  const port = validatePort(typeof desc === "number" ? desc : desc == null ? undefined : Object(desc).port);
  const instance = tcpPorts.getHandler(port);
  if (!instance || !("fetch" in instance)) {
    throw invalidArgument(`Http server with port ${port} not found. This is likely a bug with your code. `
      + `You should check if server.listen() was called with the same port (${port})`);
  }
  return await instance.fetch(request, env, ctx);
}

// Routes an inbound socket to the net.Server listening on the port it arrived
// on; resolves when the connection is finished.
export async function handleAsNodeConnection(socket: Socket, env: unknown, ctx: ExecutionContext): Promise<void> {
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
    async connect(socket: Socket, env: unknown, ctx: ExecutionContext): Promise<void> {
      await handleAsNodeConnection(socket, env, ctx);
    },
  };
}

export function httpServerHandler(desc: unknown) {
  const port = resolvePort(desc);
  return {
    async fetch(request: Request, env: unknown, ctx: ExecutionContext): Promise<Response> {
      return await handleAsNodeRequest({ port }, request, env, ctx);
    },
  };
}
