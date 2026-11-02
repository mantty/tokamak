import { decodeDevice, encodeDevice, type DeviceMessage } from "./fixtures/native-device.mts";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { test, type TestContext } from "node:test";
import { fileURLToPath } from "node:url";
import { isDeepStrictEqual } from "node:util";

import type { PluginConfig } from "@cloudflare/vite-plugin";
import { createBuilder, createServer, resolveConfig, type InlineConfig, type Plugin } from "vite";
import WebSocket from "ws";

import { tokamak } from "./dist/vite.mjs";
import { temporaryProject, workerTypes } from "./worker-fixtures.mts";

function project(t: TestContext, cloudflareOptions: PluginConfig = {}) {
  const events = fileURLToPath(import.meta.resolve("@tokamakdev/tok/events"));
  const sdk = fileURLToPath(import.meta.resolve("@tokamakdev/tok"));
  const context = fileURLToPath(import.meta.resolve("@tokamakdev/plugin/worker"));
  const root = temporaryProject(t, "tokamak-native-vite-", {
    "worker-configuration.d.ts": workerTypes,
    "wrangler.json": JSON.stringify({
      name: "native-app", main: "src/main.ts", compatibility_date: "2026-09-01",
      vars: { LABEL: "configured" },
    }),
    "src/main.ts": `export default {
    async fetch(request, env) {
      if (new URL(request.url).pathname === "/isolation") {
        return Response.json({ bridge: typeof globalThis[Symbol.for("tokamak.native")], loaded: globalThis.nativeLoaded ?? false, platform: globalThis.__tokamak_platform });
      }
      return Response.json(await env.TOKAMAK.describe());
    }
  };`,
    "src/listeners.ts": `import { onStart } from ${JSON.stringify(events)};
    import { nativeConnection } from ${JSON.stringify(context)};
    onStart(async (event, env, ctx) => ({
      label: env.LABEL, foreground: event.foreground, revision: "first",
      stage: JSON.parse(new TextDecoder().decode(await ctx[nativeConnection].call("tokamak", "lifecycleStage", new Uint8Array()))),
    }));`,
    "src/tokamak.ts": `import { TokamakEntrypoint, platform as loadedOn, type TokamakEvent } from ${JSON.stringify(sdk)};
    import { nativeConnection } from ${JSON.stringify(context)};
    import "./listeners";
    globalThis.nativeLoaded = true;
    export default class Native extends TokamakEntrypoint {
      async describe(): Promise<{ label: string; stage: string; platform: string | undefined; loadedOn: string | undefined }> {
        const stage = JSON.parse(new TextDecoder().decode(await this.ctx[nativeConnection].call("tokamak", "lifecycleStage", new Uint8Array())));
        return { label: this.env.LABEL, stage: stage as string, platform: this.ctx.platform, loadedOn };
      }
    }`,
  });
  const output = path.join(root, ".tokamak/vite");
  process.env.TOKAMAK_VITE_OUTPUT = output;
  process.env.TOKAMAK_BUILD_DIR = path.join(root, "build");
  process.env.TOKAMAK_SESSION_TOKEN = "native-test";
  process.env.TOKAMAK_PLATFORM = "android";
  let plugins: Plugin[];
  try {
    plugins = tokamak({ name: "Native App", version: "1.0.0" }, {
      inspectorPort: false, remoteBindings: false, persistState: false, ...cloudflareOptions,
    });
  } finally {
    delete process.env.TOKAMAK_VITE_OUTPUT;
    delete process.env.TOKAMAK_BUILD_DIR;
    delete process.env.TOKAMAK_SESSION_TOKEN;
    delete process.env.TOKAMAK_PLATFORM;
  }
  const config: InlineConfig = { root, configFile: false, logLevel: "silent", plugins };
  return { root, output, config };
}

function deployment(root: string) {
  const pointer = path.join(root, ".wrangler/deploy/config.json");
  const config = JSON.parse(fs.readFileSync(pointer, "utf8"));
  return [config.configPath, ...config.auxiliaryWorkers.map((worker: { configPath: string }) => worker.configPath)].map((file: string) => {
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
  const reserved: [string, PluginConfig["config"]][] = [
    ["TOKAMAK", { services: [{ binding: "TOKAMAK", service: "somewhere-else" }] }],
    ["MAIN", { vars: { MAIN: "shadowed" } }],
    ["TOKAMAK", { ai: { binding: "TOKAMAK" } }],
    ["MAIN", { durable_objects: { bindings: [{ name: "MAIN", class_name: "Counter" }] } }],
    ["MAIN", { queues: { producers: [{ binding: "MAIN", queue: "jobs" }] } }],
  ];
  for (const [binding, user] of reserved) {
    const { config } = project(t, { config: user });
    await assert.rejects(createBuilder(config), new RegExp(`${binding} is reserved`));
  }
});

test("the development server leaves tok's build directory unwatched, beside the project's own ignored paths", async t => {
  const { root, config } = project(t);
  const { watch } = (await resolveConfig({ ...config, server: { watch: { ignored: ["**/fixtures/**"] } } }, "serve")).server;
  const ignored = watch?.ignored;
  assert.ok(Array.isArray(ignored));
  const ignores = (file: string) => ignored.some(matcher => typeof matcher === "function" && matcher(file));
  assert.ok(ignored.includes("**/fixtures/**"));
  assert.equal(ignores(path.join(root, "build")), true);
  assert.equal(ignores(path.join(root, "build", "windows", "native-app", "native-app.exe")), true);
  assert.equal(ignores(path.join(root, "src", "main.ts")), false);
  assert.equal(ignores(path.join(root, "build-tools", "generate.ts")), false);

  assert.equal((await resolveConfig({ ...project(t).config, server: { watch: null } }, "serve")).server.watch, null);
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
  const server = await createServer({ ...config, server: { host: "127.0.0.1", port: 0, watch: { usePolling: true } } });
  t.after(() => server.close());
  await server.listen();
  const report = JSON.parse(fs.readFileSync(path.join(output, "server.json"), "utf8"));
  assert.equal(report.workerName, "native-app");
  const socket = new WebSocket(`ws://127.0.0.1:${report.socketPort}`, {
    headers: { "x-tokamak-session": "native-test" },
  });
  t.after(() => socket.close());
  await new Promise((resolve, reject) => { socket.once("open", resolve); socket.once("error", reject); });
  const calls: string[] = [];
  // One event is sent at a time, so one resolver receives its reply.
  const eventId = 100;
  let reply: ((message: DeviceMessage) => void) | undefined;
  socket.on("message", data => {
    const message = decodeDevice(data);
    if (message.type === "call") {
      calls.push(`${message.plugin}.${message.method}`);
      socket.send(encodeDevice({ type: "result", id: message.id, result: { value: "background", done: true } }));
    } else if (message.id === eventId) reply?.(message);
  });
  const event = async (name = "start") => {
    const result = new Promise<DeviceMessage>(resolve => { reply = resolve; });
    socket.send(encodeDevice({ type: "event", id: eventId, name, event: { foreground: false } }));
    return result;
  };
  assert.deepEqual(await (await fetch(report.url + "isolation")).json(), { bridge: "undefined", loaded: false, platform: "android" });
  assert.deepEqual(await (await fetch(report.url)).json(), { label: "configured", stage: "background", platform: "android", loadedOn: "android" });
  for (const capabilities of [
    await fetch(report.url + "_tokamak/capabilities.js"),
    await fetch(`http://127.0.0.1:${report.socketPort}/_tokamak/capabilities.js`, { headers: { "x-tokamak-session": "native-test" } }),
  ]) {
    assert.equal(capabilities.headers.get("content-type"), "text/javascript; charset=utf-8");
    assert.equal(await capabilities.text(), "export default {};\n");
  }
  assert.deepEqual(calls, ["tokamak.lifecycleStage"]);
  assert.deepEqual((await event()).reply, { label: "configured", foreground: false, revision: "first", stage: "background" });
  const listeners = path.join(root, "src/listeners.ts");
  fs.writeFileSync(listeners, fs.readFileSync(listeners, "utf8").replace('"first"', '"second"'));
  const second = { label: "configured", foreground: false, revision: "second", stage: "background" };
  let updated: DeviceMessage | undefined;
  for (const deadline = Date.now() + 10_000; Date.now() < deadline;) {
    updated = await event();
    if (isDeepStrictEqual(updated.reply, second)) break;
    await new Promise(resolve => setTimeout(resolve, 50));
  }
  assert.deepEqual(updated?.reply, second);
  const entry = path.join(root, "src/tokamak.ts");
  fs.writeFileSync(entry, fs.readFileSync(entry, "utf8").replace(
    "async describe()",
    `async event(event: TokamakEvent) {
      return { type: event.type, data: event.data, stage: JSON.parse(new TextDecoder().decode(await this.ctx[nativeConnection].call("tokamak", "lifecycleStage", new Uint8Array()))) };
    }
    async describe()`,
  ));
  const handled = { type: "plugin.background", data: { foreground: false }, stage: "background" };
  for (const deadline = Date.now() + 10_000; Date.now() < deadline;) {
    updated = await event("plugin.background");
    if (isDeepStrictEqual(updated.reply, handled)) break;
    await new Promise(resolve => setTimeout(resolve, 50));
  }
  assert.deepEqual(updated?.reply, handled);
});
