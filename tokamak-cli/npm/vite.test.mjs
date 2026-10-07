import assert from "node:assert/strict";
import fs from "node:fs";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { after, test } from "node:test";

import { cloudflare } from "@cloudflare/vite-plugin";
import { tokamak } from "@tokamakdev/tok/vite";
import { createBuilder, createServer } from "vite";

const WORKER = `export default {
  fetch() {
    return new Response("worker");
  },
};
`;

const roots = [];
after(() => {
  for (const root of roots) fs.rmSync(root, { recursive: true, force: true });
});

/** A Worker project in a new directory, with `files` relative to its root and the plugin's `options`. */
function project(files = {}, options) {
  const root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), "tokamak-vite-")));
  roots.push(root);
  const all = {
    "wrangler.jsonc": JSON.stringify({ name: "app", main: "src/index.ts", compatibility_date: "2026-09-01" }),
    "src/index.ts": WORKER,
    ...files,
  };
  for (const [name, contents] of Object.entries(all)) {
    fs.mkdirSync(path.dirname(path.join(root, name)), { recursive: true });
    fs.writeFileSync(path.join(root, name), contents);
  }
  return { root, options, output: path.join(root, "build/.tokamak/vite") };
}

/** The tokamak plugin as `tok` activates it for `app`. */
function activeTokamak(app) {
  process.env.TOKAMAK_VITE_OUTPUT = app.output;
  try {
    return tokamak(app.options);
  } finally {
    delete process.env.TOKAMAK_VITE_OUTPUT;
  }
}

/** Vite options for `app` with Cloudflare's plugin and the active tokamak plugin. */
function viteConfig(app, vite = {}) {
  return { root: app.root, configFile: false, logLevel: "silent", plugins: [cloudflare(), activeTokamak(app)], ...vite };
}

async function build(app, vite) {
  const builder = await createBuilder(viteConfig(app, vite));
  await builder.buildApp();
}

/** Runs `use` with the development server of `app`, listening on any port. */
async function serve(app, use) {
  const server = await createServer(viteConfig(app, { server: { port: 0 } }));
  try {
    await server.listen();
    return await use(server);
  } finally {
    await server.close();
  }
}

async function withEnvironment(values, run) {
  const previous = Object.fromEntries(Object.keys(values).map((name) => [name, process.env[name]]));
  Object.assign(process.env, values);
  try {
    return await run();
  } finally {
    for (const [name, value] of Object.entries(previous)) {
      if (value === undefined) delete process.env[name];
      else process.env[name] = value;
    }
  }
}

function readOutput(app, name) {
  return JSON.parse(fs.readFileSync(path.join(app.output, name), "utf8"));
}

/** The source of the Worker the build of `app` generated. */
function builtWorker(app) {
  const generated = path.join(app.root, "dist/app");
  const { main } = JSON.parse(fs.readFileSync(path.join(generated, "wrangler.json"), "utf8"));
  return fs.readFileSync(path.join(generated, main), "utf8");
}

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

test("reports an empty configuration without options, and builds without a module", async () => {
  const app = project();
  await build(app);
  assert.deepEqual(readOutput(app, "config.json").config, {});
  assert.match(builtWorker(app), /"worker"/);
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
  assert.match(worker, /export \{[^}]*\bCounter\b[^}]*\}/);
  assert.match(worker, /export \{[^}]*\bnamed\b[^}]*\}/);
  assert.match(worker, /export \{[^}]*\bas default\b[^}]*\}/);
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
    const { url } = readOutput(app, "server.json");
    assert.equal(url, server.resolvedUrls.local[0]);
    assert.notEqual(new URL(url).port, String(port));
    assert.equal(await (await fetch(url)).text(), "worker");
  } finally {
    await server.close();
    taken.close();
  }
});

test("reports the Worker's top-level name with the development server", async () => {
  for (const environment of [{}, { CLOUDFLARE_ENV: "staging" }]) {
    const app = project({
      "wrangler.jsonc": JSON.stringify({
        name: "app",
        main: "src/index.ts",
        compatibility_date: "2026-09-01",
        env: { staging: {} },
      }),
    });
    await withEnvironment(environment, () =>
      serve(app, () => assert.equal(readOutput(app, "server.json").workerName, "app")),
    );
  }
});

test("reports again when the development server restarts", async () => {
  const app = project({}, { name: "App" });
  await serve(app, async (server) => {
    fs.rmSync(app.output, { recursive: true });
    await server.restart();
    assert.deepEqual(readOutput(app, "config.json").config, { name: "App" });
    assert.equal(readOutput(app, "server.json").url, server.resolvedUrls.local[0]);
  });
});

test("fails a build in which no environment builds the entry Worker", async () => {
  const app = project({ "src/tokamak.ts": "" });
  const withoutManifest = { name: "without-manifest", configEnvironment: () => ({ build: { manifest: false } }) };
  await assert.rejects(
    build(app, { plugins: [cloudflare(), activeTokamak(app), withoutManifest] }),
    /no environment builds the entry Worker/,
  );
});
