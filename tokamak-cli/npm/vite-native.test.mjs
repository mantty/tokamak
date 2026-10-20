import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

import { createBuilder, createServer } from "vite";
import WebSocket from "ws";

import { tokamak } from "./dist/vite.mjs";
import { workerTypes } from "./worker-fixtures.mjs";

function project(t, cloudflareOptions = {}) {
  const root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), "tokamak-native-vite-")));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  fs.mkdirSync(path.join(root, "src"));
  fs.writeFileSync(path.join(root, "worker-configuration.d.ts"), workerTypes);
  fs.writeFileSync(path.join(root, "wrangler.json"), JSON.stringify({
    name: "native-app", main: "src/main.ts", compatibility_date: "2026-09-01",
    vars: { LABEL: "configured" },
  }));
  fs.writeFileSync(path.join(root, "src/main.ts"), `export default {
    async fetch(request, env) {
      if (new URL(request.url).pathname === "/isolation") {
        return Response.json({ bridge: typeof globalThis.__tokamakNativeCall, loaded: globalThis.nativeLoaded ?? false });
      }
      return Response.json(await env.TOKAMAK.describe());
    }
  };`);
  const events = fileURLToPath(import.meta.resolve("@tokamakdev/tok/events"));
  fs.writeFileSync(path.join(root, "src/listeners.ts"), `import { onStart } from ${JSON.stringify(events)};
    onStart((event, env) => ({ label: env.LABEL, foreground: event.foreground, revision: "first" }));`);
  fs.writeFileSync(path.join(root, "src/tokamak.ts"), `import { WorkerEntrypoint } from "cloudflare:workers";
    import "./listeners";
    globalThis.nativeLoaded = true;
    export default class Native extends WorkerEntrypoint {
      async describe(): Promise<{ label: string; stage: string }> {
        const stage = await globalThis.__tokamakNativeCall("tokamak", "lifecycleStage");
        return { label: this.env.LABEL, stage };
      }
    }`);
  const output = path.join(root, ".tokamak/vite");
  process.env.TOKAMAK_VITE_OUTPUT = output;
  process.env.TOKAMAK_SESSION_TOKEN = "native-test";
  let plugins;
  try {
    plugins = tokamak({ name: "Native App", version: "1.0.0" }, {
      inspectorPort: false, remoteBindings: false, persistState: false, ...cloudflareOptions,
    });
  } finally {
    delete process.env.TOKAMAK_VITE_OUTPUT;
    delete process.env.TOKAMAK_SESSION_TOKEN;
  }
  return { root, output, config: { root, configFile: false, logLevel: "silent", plugins } };
}

function deployment(root) {
  const pointer = path.join(root, ".wrangler/deploy/config.json");
  const config = JSON.parse(fs.readFileSync(pointer, "utf8"));
  return [config.configPath, ...config.auxiliaryWorkers.map(worker => worker.configPath)].map(file => {
    const absolute = path.resolve(path.dirname(pointer), file);
    const config = JSON.parse(fs.readFileSync(absolute, "utf8"));
    return { config, source: fs.readFileSync(path.resolve(path.dirname(absolute), config.main), "utf8") };
  });
}

test("the public Vite plugin builds separate portable and native workers", async t => {
  const { root, output, config } = project(t);
  await (await createBuilder(config)).buildApp();
  const [main, native] = deployment(root);
  assert.deepEqual(main.config.services, [{ binding: "TOKAMAK", service: "native-app-tokamak" }]);
  assert.deepEqual(native.config.services, [{ binding: "MAIN", service: "native-app" }]);
  assert.doesNotMatch(main.source, /nativeLoaded = true/);
  assert.match(native.source, /nativeLoaded = true/);
  assert.match(native.source, /TokamakEvents/);
  assert.deepEqual(JSON.parse(fs.readFileSync(path.join(output, "config.json"), "utf8")), {
    root, config: { name: "Native App", version: "1.0.0" },
  });
});

test("Cloudflare configuration customizations apply to both worker bundles", async t => {
  const { root, config } = project(t, {
    config(worker) {
      worker.vars.MUTATED = "mutation";
      return { name: "configured-app", vars: { RETURNED: "return value" } };
    },
  });
  await (await createBuilder(config)).buildApp();
  const [main, native] = deployment(root);
  assert.equal(main.config.name, "configured-app");
  assert.deepEqual(main.config.services, [{ binding: "TOKAMAK", service: "configured-app-tokamak" }]);
  assert.deepEqual(native.config.services, [{ binding: "MAIN", service: "configured-app" }]);
  assert.deepEqual(native.config.vars, { LABEL: "configured", MUTATED: "mutation", RETURNED: "return value" });
});

test("a user binding cannot replace the generated native worker binding", async t => {
  const { config } = project(t, {
    config: { services: [{ binding: "TOKAMAK", service: "somewhere-else" }] },
  });
  await assert.rejects(createBuilder(config), /TOKAMAK is reserved/);
});

test("development routes device calls and reloaded events through the native worker", { timeout: 30_000 }, async t => {
  const { root, output, config } = project(t);
  const registry = path.join(root, "registry");
  const previous = process.env.MINIFLARE_REGISTRY_PATH;
  process.env.MINIFLARE_REGISTRY_PATH = registry;
  t.after(() => {
    if (previous === undefined) delete process.env.MINIFLARE_REGISTRY_PATH;
    else process.env.MINIFLARE_REGISTRY_PATH = previous;
  });
  const server = await createServer({ ...config, server: { port: 0, watch: { usePolling: true } } });
  t.after(() => server.close());
  await server.listen();
  const report = JSON.parse(fs.readFileSync(path.join(output, "server.json"), "utf8"));
  assert.equal(report.workerName, "native-app");
  const socket = new WebSocket(`ws://127.0.0.1:${report.socketPort}`, {
    headers: { "x-tokamak-session": "native-test" },
  });
  t.after(() => socket.close());
  await new Promise((resolve, reject) => { socket.once("open", resolve); socket.once("error", reject); });
  const calls = [];
  const pending = new Map();
  socket.on("message", data => {
    const message = JSON.parse(String(data));
    if (message.type === "call") {
      calls.push(`${message.plugin}.${message.method}`);
      socket.send(JSON.stringify({ type: "result", id: message.id, result: { value: "background", done: true } }));
    } else {
      pending.get(message.id)?.(message);
      pending.delete(message.id);
    }
  });
  const event = async () => {
    const result = new Promise(resolve => pending.set(100, resolve));
    socket.send(JSON.stringify({ type: "event", id: 100, name: "start", event: { foreground: false } }));
    return result;
  };
  assert.deepEqual(await (await fetch(report.url + "isolation")).json(), { bridge: "undefined", loaded: false });
  assert.deepEqual(await (await fetch(report.url)).json(), { label: "configured", stage: "background" });
  assert.deepEqual(calls, ["tokamak.lifecycleStage"]);
  assert.deepEqual((await event()).reply, { label: "configured", foreground: false, revision: "first" });
  const listeners = path.join(root, "src/listeners.ts");
  fs.writeFileSync(listeners, fs.readFileSync(listeners, "utf8").replace('"first"', '"second"'));
  let updated;
  for (const deadline = Date.now() + 10_000; Date.now() < deadline;) {
    updated = await event();
    if (updated.reply?.revision === "second") break;
    await new Promise(resolve => setTimeout(resolve, 50));
  }
  assert.deepEqual(updated.reply, { label: "configured", foreground: false, revision: "second" });
});
