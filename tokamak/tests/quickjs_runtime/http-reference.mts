import { createServer, type ServerResponse } from "node:http";
import { gzipSync, deflateSync, brotliCompressSync, zstdCompressSync } from "node:zlib";
import { Miniflare, convertV4MiniflareOptions } from "miniflare";
import { esModule, listen } from "./reference.mts";

let active: ServerResponse | undefined;
let ended = false;
const compressors = new Map<string, (data: Buffer) => Buffer>([
  ["br", brotliCompressSync], ["deflate", deflateSync], ["zstd", zstdCompressSync],
]);
const upstream = createServer(async (request, response) => {
  const url = new URL(request.url ?? "/", "http://localhost");
  if (url.pathname === "/encoding") {
    const encoding = url.searchParams.get("kind") ?? "gzip";
    const compress = compressors.get(encoding) ?? gzipSync;
    response.setHeader("content-encoding", encoding);
    return response.end(compress(Buffer.from("payload")));
  }
  if (url.pathname === "/headers") {
    response.writeHead(201, "Created Here", {
      "x-latin": "caf\u00e9",
      "x-utf8": Buffer.from("\u6771\u4eac").toString("latin1"),
      "x-repeat": ["one", "two"],
      "set-cookie": ["one=1", "two=2"],
    });
    return response.end(JSON.stringify(request.headers));
  }
  if (url.pathname === "/delay") return setTimeout(() => response.end("delayed"), 100);
  if (url.pathname === "/stream") {
    ended = false;
    active = response;
    response.writeHead(200, { "content-type": "text/plain" });
    response.write("first");
    const timeout = setTimeout(() => { ended = true; response.end("last"); }, 2000);
    response.on("close", () => clearTimeout(timeout));
    return;
  }
  if (url.pathname === "/state") return response.end(JSON.stringify(ended));
  if (url.pathname === "/release") {
    ended = true;
    active?.end("last");
    return response.end("released");
  }
  if (url.pathname === "/redirect-early") {
    response.writeHead(303, { Location: "/delay" });
    return response.end();
  }
  if (url.pathname === "/early") return response.end("early");
  if (url.pathname === "/early-204") { response.writeHead(204); return response.end(); }
  if (url.pathname === "/disconnect") return setTimeout(() => request.socket.destroy(), 15);
  if (url.pathname === "/close") {
    response.writeHead(200, { connection: "close" });
    return response.end("closed", () => request.socket.end());
  }
  if (url.pathname === "/redirect") {
    response.writeHead(Number(url.searchParams.get("status")), { Location: url.searchParams.get("to") ?? "/echo" });
    return response.end();
  }
  if (url.pathname === "/loop") {
    response.writeHead(302, { Location: "/loop" });
    return response.end();
  }
  if (url.pathname === "/gzip") {
    response.writeHead(200, { "content-encoding": "gzip", "set-cookie": ["one=1", "two=2"] });
    return response.end(gzipSync("compressed"));
  }
  const chunks: Buffer[] = [];
  for await (const chunk of request) chunks.push(chunk);
  response.end(JSON.stringify({ method: request.method, body: Buffer.concat(chunks).toString() }));
});
const port = String(await listen(upstream));
const mf = new Miniflare(convertV4MiniflareOptions({
  modulesRoot: import.meta.dirname,
  cf: false,
  compatibilityDate: "2026-08-25",
  compatibilityFlags: ["nodejs_compat"],
  bindings: { FLAG: port },
  modules: [esModule(import.meta.dirname, "http.ts")],
}));
try {
  const response = await mf.dispatchFetch("http://localhost");
  console.log(JSON.stringify({ port, expected: await response.json() }));
  process.stdin.resume();
  await new Promise(resolve => process.stdin.once("end", resolve));
} finally {
  await mf.dispose();
  upstream.closeAllConnections();
  await new Promise<void>(resolve => upstream.close(() => resolve()));
}
