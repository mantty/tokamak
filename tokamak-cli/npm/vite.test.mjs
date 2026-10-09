import assert from "node:assert/strict";
import fs from "node:fs";
import { createRequire } from "node:module";
import net from "node:net";
import path from "node:path";
import { test } from "node:test";
import { fileURLToPath, pathToFileURL } from "node:url";

import { cloudflare } from "@cloudflare/vite-plugin";
import { tokamak } from "@tokamakdev/tok/vite";
import { createServer } from "vite";
import WebSocket from "ws";

import {
  WORKER,
  activeTokamak,
  build,
  builtWorker,
  project,
  readOutput,
  reported,
  serve,
  viteConfig,
  withEnvironment,
} from "./vite-fixtures.mjs";

test("adds no hooks without TOKAMAK_VITE_OUTPUT", () => {
  assert.deepEqual(tokamak({ name: "App" }), []);
});

test("reports its options, without module, and the Vite root", async () => {
  const app = project(
    { "src/setup.ts": "" },
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
const exportsName = (worker, name) => new RegExp(`export \\{[^}]*\\b${name}\\b[^}]*\\}`).test(worker);

test("reports an empty configuration without options, and builds without a module", async () => {
  const app = project();
  await build(app);
  assert.deepEqual(readOutput(app, "config.json").config, {});
  const worker = builtWorker(app);
  assert.match(worker, /"worker"/);
  assert.ok(exportsName(worker, "TokamakEvents"));
});

const ENTRY = `import { DurableObject } from "cloudflare:workers";

export class Counter extends DurableObject {}
export const named = "named";

${WORKER}`;

/** A module that marks the Worker as loaded by `name`. */
const marking = (name) => `globalThis.tokamakLoaded = ${JSON.stringify(name)};\n`;

test("imports src/tokamak.ts into the entry Worker, keeping the entry's exports", async () => {
  const app = project({
    "src/index.ts": ENTRY,
    "src/tokamak.ts": marking("tokamak.ts"),
    "src/tokamak.js": marking("tokamak.js"),
  });
  await build(app);
  const worker = builtWorker(app);
  assert.match(worker, /globalThis\.tokamakLoaded = "tokamak\.ts"/);
  assert.doesNotMatch(worker, /"tokamak\.js"/);
  for (const name of ["Counter", "named", "as default", "TokamakEvents"]) assert.ok(exportsName(worker, name), name);
});

test("imports src/tokamak.js without src/tokamak.ts", async () => {
  const app = project({ "src/tokamak.js": marking("tokamak.js") });
  await build(app);
  assert.match(builtWorker(app), /globalThis\.tokamakLoaded = "tokamak\.js"/);
});

test("imports the module the options name instead of the default", async () => {
  const app = project(
    { "src/tokamak.ts": marking("tokamak.ts"), "worker/setup.ts": marking("setup.ts") },
    { module: "worker/setup.ts" },
  );
  await build(app);
  const worker = builtWorker(app);
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
    "src/index.ts": `export default { fetch: () => new Response(globalThis.tokamakLoaded) };`,
    "src/tokamak.ts": marking("tokamak.ts"),
  });
  await serve(app, async () => {
    const response = await fetch(readOutput(app, "server.json").url);
    assert.equal(await response.text(), "tokamak.ts");
  });
});

test("reports the development server's port when the configured one is taken", async () => {
  const app = project();
  const taken = net.createServer();
  await new Promise((resolve) => taken.listen(0, "localhost", resolve));
  const port = taken.address().port;
  const server = await createServer(viteConfig(app, { server: { port } }));
  try {
    await server.listen();
    const { url } = await reported(app, "server.json");
    assert.equal(url, server.resolvedUrls.local[0]);
    assert.notEqual(new URL(url).port, String(port));
    assert.equal(await (await fetch(url)).text(), "worker");
  } finally {
    await server.close();
    taken.close();
  }
});

test("reports the Worker's name with the development server", async () => {
  const app = project();
  await serve(app, () => assert.equal(readOutput(app, "server.json").workerName, "app"));
});

test("reports again when the development server restarts", async () => {
  const app = project({}, { name: "App" });
  await serve(app, async (server) => {
    fs.rmSync(app.output, { recursive: true });
    await server.restart();
    assert.deepEqual(readOutput(app, "config.json").config, { name: "App" });
    assert.equal((await reported(app, "server.json")).url, server.resolvedUrls.local[0]);
  });
});

test("reports that a development server in middleware mode has no address", async () => {
  const app = project();
  const server = await createServer(viteConfig(app, { server: { middlewareMode: true } }));
  try {
    assert.match(readOutput(app, "server.json").error, /middleware mode.*tok dev needs Vite's own dev server/);
  } finally {
    await server.close();
  }
});

test("keeps the reported development server when a middleware-mode server starts beside it", async () => {
  const app = project();
  await serve(app, async (server) => {
    const beside = await createServer(viteConfig(app, { server: { middlewareMode: true } }));
    await beside.close();
    assert.equal(readOutput(app, "server.json").url, server.resolvedUrls.local[0]);
  });
});

const fixture = (name) => fileURLToPath(new URL(`fixtures/${name}`, import.meta.url));

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
        const { socketPort, ...report } = readOutput(app, "server.json");
        assert.deepEqual(report, { url: server.resolvedUrls.local[0], workerName: "app", certificates: certificate });
      },
      { https },
    );
  }
});

test("fails a build in which no environment builds the entry Worker", async () => {
  const app = project();
  const withoutManifest = { name: "without-manifest", configEnvironment: () => ({ build: { manifest: false } }) };
  await assert.rejects(
    build(app, { plugins: [cloudflare(), activeTokamak(app), withoutManifest] }),
    /no environment builds the entry Worker to export TokamakEvents from/,
  );
});

/**
 * A Worker project whose `file`, which `src/tokamak.ts` is or imports, replies to `start` with
 * `reply`, an expression of the event and `env`.
 */
function listeningProject(reply, file = "src/tokamak.ts") {
  const events = fileURLToPath(import.meta.resolve("@tokamakdev/tok/events"));
  return project({
    "wrangler.jsonc": JSON.stringify({ name: "app", main: "src/index.ts", compatibility_date: "2026-09-01", vars: { GREETING: "hi" } }),
    "src/tokamak.ts": `import ${JSON.stringify(`./${path.basename(file)}`)};\n`,
    [file]: `import { onStart } from ${JSON.stringify(events)};\nonStart((event, env, ctx) => {\n  ctx.waitUntil(Promise.resolve());\n  return ${reply};\n});\n`,
  });
}

/** A dev WebSocket to `port`, open, sending `token` as the device does. */
async function openSocket(port, token) {
  const socket = new WebSocket(`ws://127.0.0.1:${port}`, { headers: token ? { "x-tokamak-session": token } : {} });
  await new Promise((resolve, reject) => {
    socket.once("open", resolve);
    socket.once("error", reject);
  });
  return socket;
}

let messages = 0;

/** Sends the event `name` on `socket` and returns the message that answers it within 30 seconds. */
async function emit(socket, name, event) {
  const id = messages++;
  const answer = new Promise((resolve, reject) => {
    const unanswered = setTimeout(() => reject(new Error(`${name} was not answered`)), 30_000);
    socket.once("close", () => reject(new Error("the dev socket closed")));
    socket.on("message", function receive(data) {
      const message = JSON.parse(String(data));
      if (message.id !== id) return;
      socket.off("message", receive);
      clearTimeout(unanswered);
      resolve(message);
    });
  });
  socket.send(JSON.stringify({ type: "event", id, name, event }));
  return answer;
}

/**
 * Makes the listener in `file` reply "second" rather than "first", and returns the reply to
 * `start` once it changes, or after 10 seconds.
 */
async function replyAfterEditing(app, socket, file) {
  const source = path.join(app.root, file);
  fs.writeFileSync(source, fs.readFileSync(source, "utf8").replace('"first"', '"second"'));
  const deadline = Date.now() + 10_000;
  let reply;
  while ((reply = (await emit(socket, "start", {})).reply) !== "second" && Date.now() < deadline) {
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  return reply;
}

test("delivers dev socket events to the development Worker's listeners", async () => {
  const app = listeningProject("{ foreground: event.foreground, greeting: env.GREETING }");
  await withEnvironment({ TOKAMAK_SESSION_TOKEN: "token" }, () =>
    serve(app, async () => {
      const socket = await openSocket(readOutput(app, "server.json").socketPort, "token");
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

// The first listener's reply wins, so a stale listener would still reply "first".
for (const file of ["src/tokamak.ts", "src/listeners.ts"]) {
  test(`replaces the development Worker's listeners when ${file} changes`, async () => {
    const app = listeningProject('"first"', file);
    await withEnvironment({ TOKAMAK_SESSION_TOKEN: "token" }, () =>
      serve(app, async () => {
        const socket = await openSocket(readOutput(app, "server.json").socketPort, "token");
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

test("builds an entry Worker whose TokamakEvents runs the listeners", async () => {
  const app = listeningProject("{ foreground: event.foreground, greeting: env.GREETING }");
  await build(app);
  const generated = path.join(app.root, "dist/app");
  const { main } = JSON.parse(fs.readFileSync(path.join(generated, "wrangler.json"), "utf8"));
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
          script: `export default { fetch: async (_request, env) => Response.json(await env.APP.dispatch("start", { foreground: true })) };`,
        },
        {
          name: "app",
          modules: true,
          compatibilityDate: "2026-09-01",
          script: fs.readFileSync(path.join(generated, main), "utf8"),
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
    "src/tokamak.ts": `import { onStart } from "events-package";\nonStart(() => "pre-bundled");\n`,
  });
  const core = path.dirname(path.dirname(fileURLToPath(import.meta.resolve("@tokamakdev/plugin/events"))));
  fs.mkdirSync(path.join(app.root, "node_modules/@tokamakdev"));
  fs.symlinkSync(core, path.join(app.root, "node_modules/@tokamakdev/plugin"), "junction");
  await withEnvironment({ TOKAMAK_SESSION_TOKEN: "token" }, () =>
    serve(app, async () => {
      const socket = await openSocket(readOutput(app, "server.json").socketPort, "token");
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
      const { socketPort } = readOutput(app, "server.json");
      for (const token of [undefined, "wrong"]) {
        await assert.rejects(openSocket(socketPort, token), /401/);
      }
    }),
  );
});

/** Answers each plugin call and subscription the device receives on `socket` with `answer(message, send)`. */
function answerPluginCalls(socket, answer) {
  const send = (message) => socket.send(JSON.stringify(message));
  socket.on("message", (data) => {
    const message = JSON.parse(String(data));
    if (["call", "subscribe", "unsubscribe"].includes(message.type)) answer(message, send);
  });
}

/** The value of a plugin's result. */
const result = (id, value, done = true) => ({ type: "result", id, result: { value, done } });

test("passes the development Worker's plugin calls to the device and back", async () => {
  const events = fileURLToPath(import.meta.resolve("@tokamakdev/tok/events"));
  const app = project({
    "src/index.ts": `import { getLifecycleStage } from ${JSON.stringify(events)};\nexport default { fetch: async () => new Response(await getLifecycleStage()) };\n`,
  });
  await withEnvironment({ TOKAMAK_SESSION_TOKEN: "token" }, () =>
    serve(app, async () => {
      const { url, socketPort } = readOutput(app, "server.json");
      const socket = await openSocket(socketPort, "token");
      const calls = [];
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
    "src/tokamak.ts": `import { onStart } from ${JSON.stringify(events)};
onStart((_event, _env, ctx) => {
  const stop = globalThis.__tokamakNativeListen("location", "watchPosition", null, (position) => {
    if (position !== "stop") return void globalThis.__tokamakNativeCall("test", "received", position);
    stop();
    ctx.waitUntil(new Promise((resolve) => setTimeout(resolve, 500)).then(() => globalThis.__tokamakNativeCall("test", "stopped", null)));
  }, () => undefined);
});
`,
  });
  await withEnvironment({ TOKAMAK_SESSION_TOKEN: "token" }, () =>
    serve(app, async () => {
      const socket = await openSocket(readOutput(app, "server.json").socketPort, "token");
      const received = [];
      const waiters = new Map();
      const next = (type) => new Promise((resolve) => waiters.set(type, resolve));
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
        socket.send(JSON.stringify(result(id, { latitude: 51.5 }, false)));
        await call;
        const unsubscribed = next("unsubscribe");
        const stopped = next("call");
        socket.send(JSON.stringify(result(id, "stop", false)));

        assert.equal((await unsubscribed).id, id);
        assert.equal((await stopped).method, "stopped");
        assert.deepEqual(received, [{ latitude: 51.5 }, null]);
      } finally {
        socket.close();
      }
    }),
  );
});
