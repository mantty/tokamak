import { decodeDevice, encodeDevice, type DeviceMessage } from "./fixtures/native-device.mts";
import assert from "node:assert/strict";
import fs from "node:fs";
import { createRequire } from "node:module";
import net from "node:net";
import path from "node:path";
import { test } from "node:test";
import { fileURLToPath, pathToFileURL } from "node:url";

import { tokamak } from "@tokamakdev/tok/vite";
import { createBuilder, createServer } from "vite";
import WebSocket, { type RawData } from "ws";

import {
  WORKER,
  NATIVE_WORKER,
  build,
  builtWorker,
  project,
  readOutput,
  reported,
  serverReport,
  serve,
  viteConfig,
  withEnvironment,
  type App,
} from "./vite-fixtures.mts";
import { localURL } from "./worker-fixtures.mts";

test("builds the portable worker for a web deployment", async () => {
  const app = project({ "src/tokamak.ts": "this native module is not valid JavaScript" });
  const builder = await createBuilder({
    root: app.root, configFile: false, logLevel: "silent",
    plugins: tokamak({ name: "App" }, { inspectorPort: false, remoteBindings: false }),
  });
  await builder.buildApp();
  const deploy = JSON.parse(fs.readFileSync(path.join(app.root, ".wrangler/deploy/config.json"), "utf8"));
  assert.deepEqual(deploy.auxiliaryWorkers ?? [], []);
  assert.match(builtWorker(app), /"worker"/);
});

test("reports its options, without module, and the Vite root", async () => {
  const app = project(
    { "src/setup.ts": NATIVE_WORKER },
    {
      name: "My App",
      version: "1.2.0",
      ios: { "team-id": "TEAM", "build-number": 7, "hardened-runtime": true },
      module: "src/setup.ts",
    },
  );
  await build(app);
  assert.deepEqual(readOutput(app, "config.json"), {
    root: app.root.replaceAll("\\", "/"),
    config: {
      name: "My App",
      version: "1.2.0",
      ios: { "team-id": "TEAM", "build-number": 7, "hardened-runtime": true },
    },
  });
});

/** Whether `worker` exports `name`. */
const exportsName = (worker: string, name: string) => new RegExp(`export \\{[^}]*\\b${name}\\b[^}]*\\}`).test(worker);

test("reports an empty configuration without options, and builds without a module", async () => {
  const app = project();
  await build(app);
  assert.deepEqual(readOutput(app, "config.json").config, {});
  const worker = builtWorker(app);
  assert.match(worker, /"worker"/);
  assert.ok(exportsName(builtWorker(app, true), "TokamakEvents"));
});

const ENTRY = `import { DurableObject } from "cloudflare:workers";

export class Counter extends DurableObject {}
export const named = "named";

${WORKER}`;

/** A module that marks the Worker as loaded by `name`. */
const marking = (name: string) => `${NATIVE_WORKER}\nglobalThis.tokamakLoaded = ${JSON.stringify(name)};\n`;

test("uses src/tokamak.ts for the native worker and keeps the main worker's exports", async () => {
  const app = project({
    "src/index.ts": ENTRY,
    "src/tokamak.ts": marking("tokamak.ts"),
    "src/tokamak.js": marking("tokamak.js"),
  });
  await build(app);
  const worker = builtWorker(app);
  assert.match(builtWorker(app, true), /globalThis\.tokamakLoaded = "tokamak\.ts"/);
  assert.doesNotMatch(builtWorker(app, true), /"tokamak\.js"/);
  for (const name of ["Counter", "named", "as default"]) assert.ok(exportsName(worker, name), name);
});

test("imports src/tokamak.js without src/tokamak.ts", async () => {
  const app = project({ "src/tokamak.js": marking("tokamak.js") });
  await build(app);
  assert.match(builtWorker(app, true), /globalThis\.tokamakLoaded = "tokamak\.js"/);
});

test("imports the module the options name instead of the default", async () => {
  const app = project(
    { "src/tokamak.ts": marking("tokamak.ts"), "worker/setup.ts": marking("setup.ts") },
    { module: "worker/setup.ts" },
  );
  await build(app);
  const worker = builtWorker(app, true);
  assert.match(worker, /globalThis\.tokamakLoaded = "setup\.ts"/);
  assert.doesNotMatch(worker, /"tokamak\.ts"/);
});

test("fails when the module the options name is missing", async () => {
  const app = project({}, { module: "src/missing.ts" });
  await assert.rejects(build(app), /tokamak module not found: .*src\/missing\.ts/);
});

// Cloudflare's plugin keeps a development server's Worker exports for later
// servers in the process, so development Workers export only `default`.
test("evaluates the module in the development Worker", async () => {
  const app = project({
    "src/index.ts": `export default { fetch: async (_request, env) => new Response(await env.TOKAMAK.loaded()) };`,
    "src/tokamak.ts": `import { WorkerEntrypoint } from "cloudflare:workers";
      export default class Native extends WorkerEntrypoint { loaded(): string { return "tokamak.ts"; } }`,
  });
  await serve(app, async () => {
    const response = await fetch(serverReport(app).url);
    assert.equal(await response.text(), "tokamak.ts");
  });
});

test("reports the development server's port when the configured one is taken", async () => {
  const app = project();
  const taken = net.createServer();
  await new Promise<void>((resolve) => taken.listen(0, "127.0.0.1", resolve));
  const address = taken.address();
  assert.ok(typeof address === "object" && address !== null);
  const { port } = address;
  const server = await createServer(viteConfig(app, { server: { host: "127.0.0.1", port } }));
  try {
    await server.listen();
    await reported(app, "server.json");
    const { url } = serverReport(app);
    assert.equal(url, localURL(server));
    assert.notEqual(new URL(url).port, String(port));
    assert.equal(await (await fetch(url)).text(), "worker");
  } finally {
    await server.close();
    taken.close();
  }
});

test("reports the Worker's name with the development server", async () => {
  const app = project();
  await serve(app, () => assert.equal(serverReport(app).workerName, "app"));
});

test("reports again when the development server restarts", async () => {
  const app = project({}, { name: "App" });
  await serve(app, async (server) => {
    fs.rmSync(app.output, { recursive: true });
    await server.restart();
    assert.deepEqual(readOutput(app, "config.json").config, { name: "App" });
    await reported(app, "server.json");
    assert.equal(serverReport(app).url, localURL(server));
    assert.equal(await (await fetch(localURL(server))).text(), "worker");
  });
});

test("closes the dev socket of a development server that never listened", async () => {
  const app = project();
  const server = await createServer(viteConfig(app, { server: { host: "127.0.0.1", port: 0 } }));
  await server.close();
  assert.deepEqual(process.getActiveResourcesInfo().filter(resource => resource === "TCPServerWrap"), []);
});

test("reports that a development server in middleware mode has no address", async () => {
  const app = project();
  const server = await createServer(viteConfig(app, { server: { middlewareMode: true } }));
  try {
    const report = readOutput(app, "server.json");
    assert.ok("error" in report);
    assert.match(report.error, /middleware mode.*tok dev needs Vite's own dev server/);
  } finally {
    await server.close();
  }
});

test("keeps the reported development server when a middleware-mode server starts beside it", async () => {
  const app = project();
  await serve(app, async (server) => {
    const beside = await createServer(viteConfig(app, { server: { middlewareMode: true } }));
    await beside.close();
    assert.equal(serverReport(app).url, localURL(server));
  });
});

const fixture = (name: string) => path.join(import.meta.dirname, "fixtures", name);

test("reports the certificate an HTTPS development server serves, in each form Vite reads", async () => {
  const key = fs.readFileSync(fixture("localhost-key.pem"));
  const certificate = fs.readFileSync(fixture("localhost-cert.pem"), "utf8");
  const forms = [
    // A key and certificate in one PEM, as @vitejs/plugin-basic-ssl sets them.
    { key: `${key}${certificate}`, cert: `${key}${certificate}` },
    { key, cert: fixture("localhost-cert.pem") },
    { key: [key], cert: [Buffer.from(certificate)] },
  ];
  for (const https of forms) {
    const app = project();
    await serve(
      app,
      (server) => {
        const { socketPort, ...report } = serverReport(app);
        assert.deepEqual(report, { url: localURL(server), workerName: "app", certificates: certificate });
      },
      { https },
    );
  }
});

/**
 * A Worker project whose `file`, which `src/tokamak.ts` is or imports, replies to `start` with
 * `reply`, an expression of the event and `env`.
 */
function listeningProject(reply: string, file = "src/tokamak.ts"): App {
  const events = fileURLToPath(import.meta.resolve("@tokamakdev/tok/events"));
  const app = project({
    "wrangler.jsonc": JSON.stringify({ name: "app", main: "src/index.ts", compatibility_date: "2026-09-01", vars: { GREETING: "hi" } }),
    "src/tokamak.ts": `import ${JSON.stringify(`./${path.basename(file)}`)};\n`,
    [file]: `import { onStart } from ${JSON.stringify(events)};\nonStart((event, env, ctx) => {\n  ctx.waitUntil(Promise.resolve());\n  return ${reply};\n});\n`,
  });
  fs.appendFileSync(path.join(app.root, "src/tokamak.ts"), NATIVE_WORKER);
  return app;
}

/** A dev WebSocket to `port`, open, sending `token` as the device does. */
async function openSocket(port: number, token: string | undefined): Promise<WebSocket> {
  const socket = new WebSocket(`ws://127.0.0.1:${port}`, { headers: token ? { "x-tokamak-session": token } : {} });
  await new Promise((resolve, reject) => {
    socket.once("open", resolve);
    socket.once("error", reject);
  });
  return socket;
}

let messages = 0;

/** Sends the event `name` on `socket` and returns the message that answers it within 30 seconds. */
async function emit(socket: WebSocket, name: string, event: unknown): Promise<DeviceMessage> {
  const id = messages++;
  const answer = new Promise<DeviceMessage>((resolve, reject) => {
    const cleanup = () => {
      clearTimeout(unanswered);
      socket.off("message", receive);
      socket.off("close", closed);
    };
    const closed = () => { cleanup(); reject(new Error("the dev socket closed")); };
    const unanswered = setTimeout(() => { cleanup(); reject(new Error(`${name} was not answered`)); }, 30_000);
    function receive(data: RawData) {
      const message = decodeDevice(data);
      if (message.id !== id) return;
      cleanup();
      resolve(message);
    }
    socket.once("close", closed);
    socket.on("message", receive);
  });
  socket.send(encodeDevice({ type: "event", id, name, event }));
  return answer;
}

/**
 * Makes the listener in `file` reply "second" rather than "first", and returns the reply to
 * `start` once it changes, or after 10 seconds.
 */
async function replyAfterEditing(app: App, socket: WebSocket, file: string): Promise<unknown> {
  const source = path.join(app.root, file);
  fs.writeFileSync(source, fs.readFileSync(source, "utf8").replace('"first"', '"second"'));
  const deadline = Date.now() + 10_000;
  let reply: unknown;
  while ((reply = (await emit(socket, "start", {})).reply) !== "second" && Date.now() < deadline) {
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  return reply;
}

test("delivers dev socket events to the development Worker's listeners", async () => {
  const app = listeningProject("{ foreground: event.foreground, greeting: env.GREETING }");
  await withEnvironment({ TOKAMAK_SESSION_TOKEN: "token" }, () =>
    serve(app, async () => {
      const socket = await openSocket(serverReport(app).socketPort, "token");
      try {
        assert.deepEqual(await emit(socket, "start", { foreground: true }), {
          type: "reply",
          id: messages - 1,
          reply: { foreground: true, greeting: "hi" },
        });
        assert.equal((await emit(socket, "resume", {})).reply, null);
      } finally {
        socket.close();
      }
    }),
  );
});

test("host event handlers call the main worker through the checked binding", async () => {
  const app = listeningProject('env.MAIN.echo(event.foreground ? "main reply" : (() => "callback")).catch(error => error.name)');
  fs.writeFileSync(path.join(app.root, "src/index.ts"), `import { WorkerEntrypoint } from "cloudflare:workers";
    export default class Main extends WorkerEntrypoint {
      echo(value: string): string { return value; }
      fetch() { return new Response("main"); }
    }`);
  await withEnvironment({ TOKAMAK_SESSION_TOKEN: "token" }, () =>
    serve(app, async () => {
      const socket = await openSocket(serverReport(app).socketPort, "token");
      try {
        assert.equal((await emit(socket, "start", { foreground: true })).reply, "main reply");
        assert.equal((await emit(socket, "start", { foreground: false })).reply, "DataCloneError");
      } finally {
        socket.close();
      }
    }),
  );
});

// The first listener's reply wins, so a stale listener would still reply "first".
for (const file of ["src/tokamak.ts", "src/listeners.ts"]) {
  test(`replaces the development Worker's listeners when ${file} changes`, async () => {
    const app = listeningProject('"first"', file);
    await withEnvironment({ TOKAMAK_SESSION_TOKEN: "token" }, () =>
      serve(app, async () => {
        const socket = await openSocket(serverReport(app).socketPort, "token");
        try {
          assert.equal((await emit(socket, "start", {})).reply, "first");
          assert.equal(await replyAfterEditing(app, socket, file), "second");
        } finally {
          socket.close();
        }
      }),
    );
  });
}

test("builds a native worker whose event entrypoint runs the listeners", async () => {
  const app = listeningProject("{ foreground: event.foreground, greeting: env.GREETING }");
  await build(app);
  const require = createRequire(import.meta.resolve("@cloudflare/vite-plugin"));
  const { Miniflare, convertV4MiniflareOptions } = await import(pathToFileURL(require.resolve("miniflare")).href);
  const miniflare = new Miniflare(
    convertV4MiniflareOptions({
      workers: [
        {
          name: "caller",
          modules: true,
          compatibilityDate: "2026-09-01",
          serviceBindings: { APP: { name: "app", entrypoint: "TokamakEvents" } },
          script: `export default {
            async fetch(_request, env) {
              const { reply, listened } = await env.APP.dispatch("start", new TextEncoder().encode('{"foreground":true}'));
              return Response.json({ reply: JSON.parse(new TextDecoder().decode(reply)), listened });
            },
          };`,
        },
        {
          name: "app",
          modules: true,
          compatibilityDate: "2026-09-01",
          script: builtWorker(app, true),
          bindings: { GREETING: "hi" },
        },
      ],
    }),
  );
  try {
    const response = await miniflare.dispatchFetch("http://tokamak/");
    assert.deepEqual(await response.json(), { reply: { foreground: true, greeting: "hi" }, listened: ["start"] });
  } finally {
    await miniflare.dispose();
  }
});

test("shares the listener registry with pre-bundled dependencies", async () => {
  const app = project({
    "node_modules/events-package/package.json": JSON.stringify({ name: "events-package", type: "module", exports: "./index.js" }),
    "node_modules/events-package/index.js": `import { defineEvent } from "@tokamakdev/plugin/events";\nexport const onStart = defineEvent("start");\n`,
    "src/tokamak.ts": `${NATIVE_WORKER}\nimport { onStart } from "events-package";\nonStart(() => "pre-bundled");\n`,
  });
  const core = path.dirname(path.dirname(fileURLToPath(import.meta.resolve("@tokamakdev/plugin/events"))));
  fs.mkdirSync(path.join(app.root, "node_modules/@tokamakdev"));
  fs.symlinkSync(core, path.join(app.root, "node_modules/@tokamakdev/plugin"), "junction");
  await withEnvironment({ TOKAMAK_SESSION_TOKEN: "token" }, () =>
    serve(app, async () => {
      const socket = await openSocket(serverReport(app).socketPort, "token");
      try {
        assert.equal((await emit(socket, "start", {})).reply, "pre-bundled");
      } finally {
        socket.close();
      }
    }),
  );
});

test("refuses dev socket connections without the session token", async () => {
  const app = project();
  await withEnvironment({ TOKAMAK_SESSION_TOKEN: "token" }, () =>
    serve(app, async () => {
      const { socketPort } = serverReport(app);
      for (const token of [undefined, "wrong"]) {
        await assert.rejects(openSocket(socketPort, token), /401/);
      }
    }),
  );
});

/** Answers each plugin call and subscription the device receives on `socket` with `answer(message, send)`. */
function answerPluginCalls(socket: WebSocket, answer: (message: DeviceMessage, send: (message: DeviceMessage) => void) => void): void {
  const send = (message: DeviceMessage) => socket.send(encodeDevice(message));
  socket.on("message", (data) => {
    const message = decodeDevice(data);
    if (["call", "subscribe", "cancel"].includes(message.type)) answer(message, send);
  });
}

/** The value of a plugin's result. */
const result = (id: number | undefined, value: unknown, done = true): DeviceMessage => ({ type: "result", id, result: { value, done } });

test("passes the development Worker's plugin calls to the device and back", async () => {
  const events = fileURLToPath(import.meta.resolve("@tokamakdev/tok/events"));
  const app = project({
    "src/index.ts": `export default { fetch: async (_request, env) => new Response(await env.TOKAMAK.stage()) };`,
    "src/tokamak.ts": `import { TokamakEntrypoint } from ${JSON.stringify(fileURLToPath(import.meta.resolve("@tokamakdev/tok")))};
      import { getLifecycleStage } from ${JSON.stringify(events)};
      export default class Native extends TokamakEntrypoint { stage(): Promise<string> { return getLifecycleStage(this.ctx); } }`,
  });
  await withEnvironment({ TOKAMAK_SESSION_TOKEN: "token" }, () =>
    serve(app, async () => {
      const { url, socketPort } = serverReport(app);
      const socket = await openSocket(socketPort, "token");
      const calls: string[] = [];
      answerPluginCalls(socket, (message, send) => {
        calls.push(`${message.plugin}.${message.method}`);
        send(result(message.id, "background"));
      });
      try {
        assert.equal(await (await fetch(url)).text(), "background");
        assert.deepEqual(calls, ["tokamak.lifecycleStage"]);
      } finally {
        socket.close();
      }
    }),
  );
});

test("keeps a development Worker's listener after its event until it is removed", async () => {
  const events = fileURLToPath(import.meta.resolve("@tokamakdev/tok/events"));
  const app = project({
    "src/tokamak.ts": `const nativeCall = async (ctx, plugin, method, value = null) => JSON.parse(new TextDecoder().decode(await ctx[Symbol.for("tokamak.native")].call(plugin, method, new TextEncoder().encode(JSON.stringify(value)))));
const nativeListen = (ctx, plugin, method, value, next, error) => ctx[Symbol.for("tokamak.native")].listen(plugin, method, new TextEncoder().encode(JSON.stringify(value)), { next: payload => next(JSON.parse(new TextDecoder().decode(payload))), error });
${NATIVE_WORKER}\nimport { onStart } from ${JSON.stringify(events)};
onStart((_event, _env, ctx) => {
  const stop = nativeListen(ctx, "location", "watchPosition", null, (position) => {
    if (position !== "stop") return void nativeCall(ctx, "test", "received", position);
    stop();
    ctx.waitUntil(new Promise((resolve) => setTimeout(resolve, 500)).then(() => nativeCall(ctx, "test", "stopped", null)));
  }, () => undefined);
});
`,
  });
  await withEnvironment({ TOKAMAK_SESSION_TOKEN: "token" }, () =>
    serve(app, async () => {
      const socket = await openSocket(serverReport(app).socketPort, "token");
      const received: unknown[] = [];
      const waiters = new Map<string, (message: DeviceMessage) => void>();
      const next = (type: string) => new Promise<DeviceMessage>((resolve) => waiters.set(type, resolve));
      answerPluginCalls(socket, (message, send) => {
        if (message.type === "call") {
          received.push(message.arguments);
          send(result(message.id, null));
        }
        waiters.get(message.type)?.(message);
      });
      try {
        const subscribed = next("subscribe");
        await emit(socket, "start", { foreground: true });
        const { id } = await subscribed;
        await new Promise((resolve) => setTimeout(resolve, 1000));

        const call = next("call");
        socket.send(encodeDevice(result(id, { latitude: 51.5 }, false)));
        await call;
        const cancelled = next("cancel");
        const stopped = next("call");
        socket.send(encodeDevice(result(id, "stop", false)));

        assert.equal((await cancelled).id, id);
        assert.equal((await stopped).method, "stopped");
        assert.deepEqual(received, [{ latitude: 51.5 }, null]);
      } finally {
        socket.close();
      }
    }),
  );
});
