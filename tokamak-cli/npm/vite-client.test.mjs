import assert from "node:assert/strict";
import fs from "node:fs";
import http from "node:http";
import path from "node:path";
import { test } from "node:test";
import { pathToFileURL } from "node:url";
import { build as viteBuild } from "vite";
import WebSocket from "ws";
import { createClient } from "../../plugins/core/src/rpc/client.mjs";
import { createClientPlugin } from "./dist/client.mjs";
import { project, readOutput, serve, withEnvironment } from "./vite-fixtures.mjs";

const WORKER = `import { WorkerEntrypoint } from "cloudflare:workers";
let activeSockets = 0;
export default class Native extends WorkerEntrypoint {
  echo(value: { count: bigint; bytes: Uint8Array }): { count: bigint; bytes: Uint8Array } { return value; }
  fail(): never { throw Object.assign(new Error("Denied"), { code: "DENIED" }); }
  stream(): ReadableStream<Uint8Array> {
    return new ReadableStream({ type: "bytes", start(controller) {
      controller.enqueue(new Uint8Array([3, 4])); controller.close();
    } });
  }
  fetch(request: Request): Response {
    const path = new URL(request.url).pathname;
    if (path === "/_tokamak/active") return new Response(String(activeSockets));
    if (path === "/_tokamak/socket") {
      const [client, server] = Object.values(new WebSocketPair());
      server.accept();
      activeSockets++;
      server.addEventListener("close", () => activeSockets--);
      return new Response(null, { status: 101, webSocket: client });
    }
    if (request.method === "POST") return new Response(request.body, { headers: [
      ["Set-Cookie", "a=1"], ["Set-Cookie", "b=2"],
    ] });
    return new Response("native:" + path);
  }
}`;

async function bundledClient(app, native) {
  const entry = path.join(app.root, "client.ts");
  fs.writeFileSync(entry, 'export { default } from "virtual:tokamak/client";');
  const outDir = path.join(app.root, native ? "native-client" : "web-client");
  await viteBuild({
    root: app.root, configFile: false, logLevel: "silent",
    plugins: [createClientPlugin(undefined, native)],
    build: { outDir, lib: { entry, formats: ["es"], fileName: "client" } },
  });
  const file = path.join(outDir, "client.mjs");
  const source = fs.readFileSync(file, "utf8");
  assert.doesNotMatch(source, /cloudflare:workers|TokamakEntrypoint|nativeConnection|native:|DENIED/);
  return (await import(pathToFileURL(file).href)).default;
}

test("the web client has the declared API and reports unavailable native operations", async () => {
  const app = project({ "src/tokamak.ts": WORKER });
  const client = await bundledClient(app, false);
  assert.deepEqual(Object.keys(client).sort(), ["echo", "fail", "stream"]);
  await assert.rejects(client.echo({ count: 1n, bytes: new Uint8Array() }), { name: "NotSupportedError" });
});

test("generated clients reach the native Vite worker through the authenticated relay", { timeout: 30_000 }, async t => {
  const app = project({ "src/tokamak.ts": WORKER });
  const client = await bundledClient(app, true);
  const originalSocket = globalThis.WebSocket;
  const originalLocation = globalThis.location;
  t.after(() => {
    globalThis.WebSocket = originalSocket;
    if (originalLocation === undefined) delete globalThis.location;
    else globalThis.location = originalLocation;
  });
  await withEnvironment({ TOKAMAK_SESSION_TOKEN: "client-test" }, () => serve(app, async () => {
    const { socketPort, url } = readOutput(app, "server.json");
    globalThis.location = new URL(`http://127.0.0.1:${socketPort}/page`);
    const headers = {
      "x-tokamak-session": "client-test",
      "x-forwarded-host": "app.tokamak.local", "x-forwarded-proto": "https",
      origin: "https://app.tokamak.local",
    };
    globalThis.WebSocket = class extends WebSocket {
      constructor(url) { super(url, { headers }); }
    };
    const value = { count: 42n, bytes: new Uint8Array([1, 2]) };
    assert.deepEqual(await client.echo(value), value);
    await assert.rejects(client.fail(), { message: "Denied", code: "DENIED" });
    assert.deepEqual(new Uint8Array(await new Response(await client.stream()).arrayBuffer()), new Uint8Array([3, 4]));
    assert.equal(await (await fetch(url)).text(), "worker");
    const native = `http://127.0.0.1:${socketPort}/_tokamak/custom`;
    assert.equal(await (await fetch(native, { headers })).text(), "native:/_tokamak/custom");
    assert.equal((await fetch(native)).status, 401);
    const bytes = new Uint8Array(100_000).fill(255);
    const posted = await fetch(native, { headers, method: "POST", body: bytes });
    assert.deepEqual(new Uint8Array(await posted.arrayBuffer()), bytes);
    assert.deepEqual(posted.headers.getSetCookie(), ["a=1", "b=2"]);
    const forbidden = new WebSocket(`ws://127.0.0.1:${socketPort}/_tokamak/app`, {
      headers: { ...headers, origin: "https://unrelated.example" },
    });
    await assert.rejects(new Promise((resolve, reject) => {
      forbidden.once("open", resolve); forbidden.once("error", reject);
    }), /403/);
    await assert.rejects(createClient(["event"], `http://127.0.0.1:${socketPort}/_tokamak/app`).event(),
      { name: "TypeError", message: "Unknown RPC method: event" });
    const rejected = await new Promise((resolve, reject) => {
      const request = http.get(`http://127.0.0.1:${socketPort}/_tokamak/socket`, {
        headers: { ...headers, connection: "Upgrade", upgrade: "websocket",
          "sec-websocket-version": "13", "sec-websocket-key": "invalid" },
      }, response => {
        response.resume();
        response.once("end", () => resolve(response.statusCode));
      });
      request.once("error", reject);
    });
    assert.equal(rejected, 400);
    let active;
    for (const deadline = Date.now() + 1000; Date.now() < deadline;) {
      active = await (await fetch(`http://127.0.0.1:${socketPort}/_tokamak/active`, { headers })).text();
      if (active === "0") break;
      await new Promise(resolve => setTimeout(resolve, 10));
    }
    assert.equal(active, "0", "a rejected handshake closes its worker socket");
  }));
});
