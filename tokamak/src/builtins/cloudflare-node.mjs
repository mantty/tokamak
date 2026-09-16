function invalidServer() {
  throw new Error("No Node.js HTTP server is available for this handler");
}

function serverFor(portOrServer) {
  if (portOrServer && typeof portOrServer.__handle === "function") return portOrServer;
  const port = typeof portOrServer === "object" ? portOrServer?.port : portOrServer;
  return globalThis.__tokamak_http_servers?.get(Number(port));
}

export function httpServerHandler(portOrServer) {
  if (portOrServer === undefined) invalidServer();
  if (portOrServer?.__handle && !portOrServer.listening) portOrServer.listen();
  return { fetch: request => handleAsNodeRequest(portOrServer?.__handle ? portOrServer : portOrServer?.port ?? portOrServer, request) };
}

export function handleAsNodeRequest(port, request, env, ctx) {
  const server = serverFor(port);
  if (!server) return Promise.reject(new Error(`No Node.js HTTP server is listening on port ${port}`));
  return server.__handle(request, env, ctx);
}
