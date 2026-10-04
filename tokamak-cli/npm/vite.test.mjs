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

/** A Worker project in a new directory, with `files` relative to its root. */
function project(files = {}) {
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
  return { root, output: path.join(root, "build/.tokamak/vite") };
}

/** The tokamak plugin as `tok` activates it for `app`. */
function activeTokamak(app) {
  process.env.TOKAMAK_VITE_OUTPUT = app.output;
  try {
    return tokamak();
  } finally {
    delete process.env.TOKAMAK_VITE_OUTPUT;
  }
}

/** Vite options for `app` with Cloudflare's plugin and the active tokamak plugin. */
function viteConfig(app, vite = {}) {
  return { root: app.root, configFile: false, logLevel: "silent", plugins: [cloudflare(), activeTokamak(app)], ...vite };
}

/** Builds `app` with `environment` set. */
async function build(app, { vite, environment = {} } = {}) {
  await withEnvironment(environment, async () => {
    const builder = await createBuilder(viteConfig(app, vite));
    await builder.buildApp();
  });
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

/** Resolves once `check` passes, retrying for up to five seconds. */
async function eventually(check) {
  for (let attempt = 0; ; attempt++) {
    try {
      return check();
    } catch (error) {
      if (attempt === 50) throw error;
      await new Promise((resolve) => setTimeout(resolve, 100));
    }
  }
}

test("adds no hooks without TOKAMAK_VITE_OUTPUT", () => {
  assert.deepEqual(tokamak(), []);
});

test("writes the config export of src/tokamak.ts with the app's aliases", async () => {
  const app = project({
    "src/tokamak.ts": `import type { Config } from "@tokamakdev/tok";
import { base } from "@shared/base";

export const config = {
  ...base,
  name: "My App",
  ios: { "team-id": "TEAM", "build-number": 7, "hardened-runtime": true },
} satisfies Config;
`,
    "shared/base.ts": `export const base = { version: "1.2.0" };`,
  });
  await build(app, { vite: { resolve: { alias: { "@shared": path.join(app.root, "shared") } } } });
  assert.deepEqual(readOutput(app, "config.json"), {
    file: path.join(app.root, "src/tokamak.ts").replaceAll("\\", "/"),
    config: {
      version: "1.2.0",
      name: "My App",
      ios: { "team-id": "TEAM", "build-number": 7, "hardened-runtime": true },
    },
  });
});

test("reads src/tokamak.js in the environment it is built in", async () => {
  const app = project({ "src/tokamak.js": `export const config = { version: process.env.APP_VERSION };` });
  await build(app, { environment: { APP_VERSION: "3.0.0" } });
  assert.deepEqual(readOutput(app, "config.json").config, { version: "3.0.0" });
});

test("reads the configuration file TOKAMAK_CONFIG names", async () => {
  const app = project({
    "src/tokamak.ts": `export const config = { name: "Default" };`,
    "config/test.ts": `export const config = { name: "Test" };`,
  });
  await build(app, { environment: { TOKAMAK_CONFIG: path.join(app.root, "config/test.ts") } });
  assert.deepEqual(readOutput(app, "config.json").config, { name: "Test" });
});

test("fails when the configuration file TOKAMAK_CONFIG names is missing", async () => {
  const app = project();
  await assert.rejects(build(app, { environment: { TOKAMAK_CONFIG: "missing.ts" } }), /configuration file not found/);
});

test("writes an empty configuration without a configuration file or config export", async () => {
  const missing = project();
  await build(missing);
  assert.deepEqual(readOutput(missing, "config.json"), { config: {} });

  const empty = project({ "src/tokamak.ts": `export const other = 1;` });
  await build(empty);
  assert.deepEqual(readOutput(empty, "config.json").config, {});
});

test("reports the development server's port when the configured one is taken", async () => {
  const app = project({ "src/tokamak.ts": `export const config = { name: "Dev" };` });
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

test("rewrites the configuration in development when a file it imports changes", async () => {
  const app = project({
    "src/tokamak.ts": `import { name } from "./name";\nexport const config = { name };`,
    "src/name.ts": `export const name = "Before";`,
  });
  const server = await createServer(viteConfig(app, { server: { port: 0 } }));
  try {
    await server.listen();
    assert.deepEqual(readOutput(app, "config.json").config, { name: "Before" });
    fs.writeFileSync(path.join(app.root, "src/name.ts"), `export const name = "After";`);
    await eventually(() => assert.deepEqual(readOutput(app, "config.json").config, { name: "After" }));
  } finally {
    await server.close();
  }
});

const ENTRY = `import { DurableObject } from "cloudflare:workers";

export class Counter extends DurableObject {}
export const named = "named";

export default {
  fetch() {
    return new Response(JSON.stringify({ loaded: globalThis.tokamakLoaded, evaluated: globalThis.configEvaluated }));
  },
};
`;

const TOKAMAK = `globalThis.tokamakLoaded = "loaded";

const secret = () => "SECRET-TEAM";

export const config = (globalThis.configEvaluated = true, { ios: { "team-id": secret() } });
`;

function entryProject() {
  return project({ "src/index.ts": ENTRY, "src/tokamak.ts": TOKAMAK });
}

/** The source of the Worker the build of `app` generated. */
function builtWorker(app) {
  const generated = path.join(app.root, "dist/app");
  const { main } = JSON.parse(fs.readFileSync(path.join(generated, "wrangler.json"), "utf8"));
  return fs.readFileSync(path.join(generated, main), "utf8");
}

test("imports the configuration file into the entry Worker, keeping the entry's exports", async () => {
  const app = entryProject();
  await build(app);
  const worker = builtWorker(app);
  assert.match(worker, /globalThis\.tokamakLoaded = "loaded"/);
  assert.match(worker, /export \{[^}]*\bCounter\b[^}]*\}/);
  assert.match(worker, /export \{[^}]*\bnamed\b[^}]*\}/);
  assert.match(worker, /export \{[^}]*\bas default\b[^}]*\}/);
});

test("keeps the config export out of the Worker bundle", async () => {
  const app = entryProject();
  await build(app);
  const worker = builtWorker(app);
  assert.doesNotMatch(worker, /SECRET-TEAM|configEvaluated = true/);
  assert.deepEqual(readOutput(app, "config.json").config, { ios: { "team-id": "SECRET-TEAM" } });
});

test("evaluates the configuration file, without its config export, in the development Worker", async () => {
  const app = entryProject();
  const server = await createServer(viteConfig(app, { server: { port: 0 } }));
  try {
    await server.listen();
    const response = await fetch(readOutput(app, "server.json").url);
    assert.deepEqual(await response.json(), { loaded: "loaded" });
  } finally {
    await server.close();
  }
});

test("fails on a config export it cannot remove", async () => {
  const app = project({ "src/tokamak.ts": `const config = { name: "App" };\nexport { config };\n` });
  await assert.rejects(build(app), /declare config in its own `export const config = \.\.\.` statement/);
});
