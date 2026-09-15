import { readFileSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { gzipSync } from "node:zlib";
import { Miniflare, convertV4MiniflareOptions } from "miniflare";

const payload = gzipSync(Buffer.from("upstream-payload"));
const upstream = createServer((request, response) => {
  response.setHeader("Content-Type", "text/plain");
  response.setHeader("Set-Cookie", ["a=1", "b=2"]);
  response.setHeader("Content-Encoding", "gzip");
  response.setHeader("Content-Length", payload.length);
  response.end(payload);
});
await new Promise(resolve => upstream.listen(0, "127.0.0.1", resolve));

const root = import.meta.dirname;
const state = await mkdtemp(join(tmpdir(), "tokamak-boundary-"));
process.chdir(state);
const mf = new Miniflare(convertV4MiniflareOptions({
  modulesRoot: root,
  cf: false,
  compatibilityDate: "2026-08-25",
  compatibilityFlags: ["nodejs_compat"],
  bindings: { UPSTREAM_PORT: String(upstream.address().port) },
  modules: [{
    type: "ESModule",
    path: `${root}/boundary.mjs`,
    contents: readFileSync(`${root}/boundary.mjs`, "utf8"),
  }],
}));
try {
  const results = {};
  const echo = await mf.dispatchFetch("http://localhost/echo", {
    method: "POST",
    body: "x".repeat(70000),
    headers: { "content-type": "application/octet-stream" },
  });
  results.echo = await echo.json();
  const stream = await mf.dispatchFetch("http://localhost/stream");
  results.stream = {
    body: await stream.text(),
    hasContentLength: stream.headers.has("content-length"),
  };
  const egress = await mf.dispatchFetch("http://localhost/egress");
  results.egress = await egress.json();
  console.log(JSON.stringify(results));
} finally {
  await mf.dispose();
  upstream.close();
  await rm(state, { recursive: true, force: true });
}
