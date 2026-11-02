// Projects and Vite servers for the tokamak Vite plugin's tests.
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { after, type TestContext } from "node:test";

import { tokamak, type TokamakOptions } from "@tokamakdev/tok/vite";
import { createBuilder, createServer, type InlineConfig, type ServerOptions, type ViteDevServer } from "vite";
import type { ConfigReport, ServerFile, ServerReport } from "./src/contracts.generated.mts";
import { workerTypes, writeFiles } from "./worker-fixtures.mts";

/** A project for the plugin, with the plugin's options and the directory it reports to. */
export interface App {
  root: string;
  options?: TokamakOptions;
  output: string;
}

/** The reports the plugin writes, by file name. */
interface Reports {
  "config.json": ConfigReport;
  "server.json": ServerFile;
}

export const WORKER = `export default {
  fetch() {
    return new Response("worker");
  },
};
`;

export const NATIVE_WORKER = `import { WorkerEntrypoint } from "cloudflare:workers";
export default class Native extends WorkerEntrypoint {}
`;

const roots: string[] = [];
after(() => {
  for (const root of roots) fs.rmSync(root, { recursive: true, force: true });
});

// Development Workers register in a dev registry of the tests' own.
process.env.MINIFLARE_REGISTRY_PATH = fs.mkdtempSync(path.join(os.tmpdir(), "tokamak-registry-"));
roots.push(process.env.MINIFLARE_REGISTRY_PATH);

/** A Worker project in a new directory, with `files` relative to its root and the plugin's `options`. */
export function project(files: Record<string, string> = {}, options?: TokamakOptions): App {
  const root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), "tokamak-vite-")));
  roots.push(root);
  writeFiles(root, {
    "wrangler.jsonc": JSON.stringify({ name: "app", main: "src/index.ts", compatibility_date: "2026-09-01" }),
    "src/index.ts": WORKER,
    "worker-configuration.d.ts": workerTypes,
    ...files,
  });
  return { root, options, output: path.join(root, "build/.tokamak/vite") };
}

/** The tokamak plugin as `tok` activates it for `app`. */
export function activeTokamak(app: App) {
  process.env.TOKAMAK_VITE_OUTPUT = app.output;
  try {
    return tokamak(app.options, { inspectorPort: false, remoteBindings: false, persistState: false });
  } finally {
    delete process.env.TOKAMAK_VITE_OUTPUT;
  }
}

/** Vite options for `app` with Cloudflare's plugin and the active tokamak plugin. */
export function viteConfig(app: App, vite: InlineConfig = {}): InlineConfig {
  return { root: app.root, configFile: false, logLevel: "silent", plugins: activeTokamak(app), ...vite };
}

export async function build(app: App, vite?: InlineConfig): Promise<void> {
  const builder = await createBuilder(viteConfig(app, vite));
  await builder.buildApp();
}

/**
 * Runs `use` with the development server of `app`, listening on any port,
 * with Vite's `server` options, once it is reported.
 */
export async function serve<T>(app: App, use: (server: ViteDevServer) => T | Promise<T>, options: ServerOptions = {}): Promise<T> {
  const server = await createServer(viteConfig(app, { server: { host: "127.0.0.1", port: 0, watch: { usePolling: true }, ...options } }));
  try {
    await server.listen();
    await reported(app, "server.json");
    return await use(server);
  } finally {
    await server.close();
  }
}

export async function withEnvironment<T>(values: Record<string, string>, run: () => T | Promise<T>): Promise<T> {
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

export function readOutput<Name extends keyof Reports>(app: App, name: Name): Reports[Name] {
  return JSON.parse(fs.readFileSync(path.join(app.output, name), "utf8"));
}

/** The plugin's server.json, which reports a server and its URL. */
export function serverReport(app: App): ServerReport {
  const report = readOutput(app, "server.json");
  if ("error" in report) throw new Error(report.error);
  return report;
}

/** Sets the global `name` to `value` until `t` ends, as a page provides the generated clients' `location` and `WebSocket`. */
export function setGlobal(t: TestContext, name: string, value: unknown): void {
  const previous = Object.getOwnPropertyDescriptor(globalThis, name);
  Object.defineProperty(globalThis, name, { value, configurable: true, writable: true });
  t.after(() => {
    if (previous) Object.defineProperty(globalThis, name, previous);
    else Reflect.deleteProperty(globalThis, name);
  });
}

/** Waits until the plugin has written its report `name`. */
export async function reported(app: App, name: keyof Reports): Promise<void> {
  for (const deadline = Date.now() + 5_000; !fs.existsSync(path.join(app.output, name)); ) {
    if (Date.now() > deadline) throw new Error(`the plugin did not write ${name}`);
    await new Promise((resolve) => setTimeout(resolve, 10));
  }
}

/** The source of the Worker the build of `app` generated. */
export function builtWorker(app: App, native = false): string {
  const pointer = path.join(app.root, ".wrangler/deploy/config.json");
  const deploy = JSON.parse(fs.readFileSync(pointer, "utf8"));
  const config = path.resolve(path.dirname(pointer), native ? deploy.auxiliaryWorkers[0].configPath : deploy.configPath);
  const { main } = JSON.parse(fs.readFileSync(config, "utf8"));
  return fs.readFileSync(path.resolve(path.dirname(config), main), "utf8");
}
