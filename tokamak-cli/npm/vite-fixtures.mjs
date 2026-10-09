// Projects and Vite servers for the tokamak Vite plugin's tests.
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { after } from "node:test";

import { cloudflare } from "@cloudflare/vite-plugin";
import { tokamak } from "@tokamakdev/tok/vite";
import { createBuilder, createServer } from "vite";

export const WORKER = `export default {
  fetch() {
    return new Response("worker");
  },
};
`;

const roots = [];
after(() => {
  for (const root of roots) fs.rmSync(root, { recursive: true, force: true });
});

// Development Workers register in a dev registry of the tests' own.
process.env.MINIFLARE_REGISTRY_PATH = fs.mkdtempSync(path.join(os.tmpdir(), "tokamak-registry-"));
roots.push(process.env.MINIFLARE_REGISTRY_PATH);

/** A Worker project in a new directory, with `files` relative to its root and the plugin's `options`. */
export function project(files = {}, options) {
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
export function activeTokamak(app) {
  process.env.TOKAMAK_VITE_OUTPUT = app.output;
  try {
    return tokamak(app.options);
  } finally {
    delete process.env.TOKAMAK_VITE_OUTPUT;
  }
}

/** Vite options for `app` with Cloudflare's plugin and the active tokamak plugin. */
export function viteConfig(app, vite = {}) {
  return { root: app.root, configFile: false, logLevel: "silent", plugins: [cloudflare(), activeTokamak(app)], ...vite };
}

export async function build(app, vite) {
  const builder = await createBuilder(viteConfig(app, vite));
  await builder.buildApp();
}

/** Runs `use` with the development server of `app`, listening on any port, once it is reported. */
export async function serve(app, use) {
  const server = await createServer(viteConfig(app, { server: { port: 0 } }));
  try {
    await server.listen();
    await reported(app, "server.json");
    return await use(server);
  } finally {
    await server.close();
  }
}

export async function withEnvironment(values, run) {
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

export function readOutput(app, name) {
  return JSON.parse(fs.readFileSync(path.join(app.output, name), "utf8"));
}

/** The plugin's report `name`, once it has written it. */
export async function reported(app, name) {
  for (const deadline = Date.now() + 5_000; !fs.existsSync(path.join(app.output, name)); ) {
    if (Date.now() > deadline) throw new Error(`the plugin did not write ${name}`);
    await new Promise((resolve) => setTimeout(resolve, 10));
  }
  return readOutput(app, name);
}

/** The source of the Worker the build of `app` generated. */
export function builtWorker(app) {
  const generated = path.join(app.root, "dist/app");
  const { main } = JSON.parse(fs.readFileSync(path.join(generated, "wrangler.json"), "utf8"));
  return fs.readFileSync(path.join(generated, main), "utf8");
}
