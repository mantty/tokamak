import { X509Certificate } from "node:crypto";
import fs from "node:fs";
import type { ServerOptions as HttpsServerOptions } from "node:https";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  normalizePath,
  type EnvironmentModuleNode,
  type Plugin,
  type ResolvedBuildEnvironmentOptions,
} from "vite";

import { openDevSocket } from "./dev-socket.mjs";
import type { Config } from "./index.mjs";

/** Modules tried in order when the options name none. */
const DEFAULT_MODULES = ["src/tokamak.ts", "src/tokamak.js"];

/** Wrangler configuration files Cloudflare's plugin tries in order. */
const WRANGLER_FILES = ["wrangler.jsonc", "wrangler.json", "wrangler.toml"];

const MIDDLEWARE_MODE_ERROR =
  "Vite is running in middleware mode, so tokamak cannot find the development server's address; tok dev needs Vite's own dev server";

/**
 * Reports the app's tokamak configuration, and the development server's
 * address, HTTPS certificate, Worker name and dev socket port, to `tok`, and
 * makes the entry Worker import `module` and export `TokamakEvents`. Without
 * `TOKAMAK_VITE_OUTPUT`, which `tok` sets, it adds no hooks.
 */
export function tokamak({
  module,
  ...config
}: Config & {
  /**
   * The module the entry Worker imports, relative to the Vite root; by
   * default `src/tokamak.ts`, then `src/tokamak.js`, when one exists.
   */
  module?: string;
} = {}): Plugin[] {
  const output = process.env.TOKAMAK_VITE_OUTPUT;
  if (!output) {
    return [];
  }
  let file: string | undefined;
  const events = import.meta.resolve("@tokamakdev/plugin/events");
  /** The module that defines `TokamakEvents`. */
  const entrypoint = normalizePath(fileURLToPath(new URL("./entrypoint.js", events)));
  /** The module that holds event listeners. */
  const registryFile = normalizePath(fileURLToPath(new URL("./registry.ts", events)));
  /** The module IDs of each entry environment's inputs. */
  const entryIds = new Map<string, Promise<(string | undefined)[]>>();
  /** The entry environments whose input exports `TokamakEvents`. */
  const exporting = new Set<string>();
  return [
    {
      name: "tokamak",
      // Keeps @tokamakdev/plugin out of pre-bundled dependencies, so they
      // share the one module that holds event listeners.
      configEnvironment() {
        return { optimizeDeps: { exclude: ["@tokamakdev/plugin"] } };
      },
      configResolved({ root }) {
        file = findModule(root, module);
        writeJson(output, "config.json", { root, config });
      },
      async configureServer(server) {
        const httpServer = server.httpServer;
        // A middleware-mode server beside a reported one, such as the server
        // Astro syncs content with, leaves the report in place.
        if (!httpServer) {
          if (!fs.existsSync(path.join(output, "server.json"))) {
            writeJson(output, "server.json", { error: MIDDLEWARE_MODE_ERROR });
          }
          return;
        }
        const worker = await readWorker(server.config.root);
        httpServer.once("listening", () => {
          openDevSocket(worker?.name, process.env.TOKAMAK_SESSION_TOKEN).then(
            (socket) => {
              httpServer.once("close", () => void socket.close());
              const urls = server.resolvedUrls;
              writeJson(output, "server.json", {
                url: urls?.local[0] ?? urls?.network[0],
                workerName: worker?.topLevelName ?? worker?.name,
                certificates: endEntityCertificates(server.config.server.https),
                socketPort: socket.port,
              });
            },
            (error: unknown) => server.config.logger.error(`tokamak could not open its dev socket: ${String(error)}`),
          );
        });
      },
      async buildApp(builder) {
        const environments = Object.values(builder.environments);
        if (!environments.some((environment) => buildsEntryWorker(environment.config))) {
          throw new Error("no environment builds the entry Worker to export TokamakEvents from");
        }
      },
    },
    {
      name: "tokamak:entry",
      applyToEnvironment: ({ config }) => buildsEntryWorker(config),
      async transform(code, id) {
        const environment = this.environment;
        const ids =
          entryIds.get(environment.name) ??
          Promise.all(inputs(environment.config.build).map(async (input) => (await this.resolve(input))?.id));
        entryIds.set(environment.name, ids);
        if (!(await ids).includes(id)) {
          return;
        }
        exporting.add(environment.name);
        // In Cloudflare's Worker entry, which dev and build both start from,
        // this follows the imports of its polyfills and of the app's entry.
        const imports = file ? `import ${JSON.stringify(file)};\n` : "";
        return { code: `${code}\n${imports}export { TokamakEvents } from ${JSON.stringify(entrypoint)};\n`, map: null };
      },
      // A module that registers listeners adds them again each time it
      // evaluates, so when an update re-evaluates one, the registry and every
      // module that imports it evaluate again too.
      async hotUpdate({ modules, timestamp }) {
        const graph = this.environment.moduleGraph;
        const registry = graph.getModuleById(registryFile);
        if (!registry) {
          return;
        }
        const entries = (await entryIds.get(this.environment.name)) ?? [];
        const registering = [...withImporters(registry)].filter((module) => !entries.some((id) => id === module.id));
        const updated = new Set(modules.flatMap((changed) => [...withImporters(changed)]));
        if (registering.some((module) => updated.has(module))) {
          graph.invalidateModule(registry, new Set(), timestamp, true);
        }
      },
      generateBundle() {
        if (!exporting.has(this.environment.name)) {
          this.error(`the input of the ${this.environment.name} environment did not export TokamakEvents`);
        }
      },
    },
  ];
}

/** `module` and every module that imports it, directly or through others. */
function withImporters(module: EnvironmentModuleNode): Set<EnvironmentModuleNode> {
  const found = new Set<EnvironmentModuleNode>();
  const pending = [module];
  for (let current = pending.pop(); current; current = pending.pop()) {
    if (!found.has(current)) {
      found.add(current);
      pending.push(...current.importers);
    }
  }
  return found;
}

/** The module `name` under `root`, or else the first default under `root` that exists. */
function findModule(root: string, name: string | undefined): string | undefined {
  const candidates = (name ? [name] : DEFAULT_MODULES).map((candidate) => normalizePath(path.resolve(root, candidate)));
  const file = candidates.find((candidate) => fs.existsSync(candidate));
  if (name && !file) {
    throw new Error(`tokamak module not found: ${candidates[0]}`);
  }
  return file;
}

/**
 * The Worker in the Wrangler configuration file in `root`, read as
 * Cloudflare's plugin reads it without `configPath`, in the `CLOUDFLARE_ENV`
 * environment: its name, and its name without the environment's suffix.
 */
async function readWorker(root: string): Promise<{ name?: string; topLevelName?: string } | undefined> {
  const file = WRANGLER_FILES.map((name) => path.join(root, name)).find((candidate) => fs.existsSync(candidate));
  if (!file) {
    return undefined;
  }
  const { unstable_readConfig } = await import("wrangler");
  return unstable_readConfig({ config: file, env: process.env.CLOUDFLARE_ENV }, { hideWarnings: true });
}

/**
 * The PEM of the end-entity certificate of each chain in `https.cert`, which
 * Vite reads from a file when a string names one.
 */
function endEntityCertificates(https: HttpsServerOptions | undefined): string | undefined {
  const cert = typeof https?.cert === "string" ? readFileIfExists(https.cert) : https?.cert;
  return cert && [cert].flat().map((chain) => new X509Certificate(chain).toString()).join("");
}

function readFileIfExists(file: string): string | Buffer {
  try {
    return fs.readFileSync(path.resolve(file));
  } catch {
    return file;
  }
}

/**
 * Whether an environment builds from the entry Worker: Cloudflare's plugin
 * gives a manifest to its entry and prerender Workers, and Astro to its server
 * and prerender environments.
 */
function buildsEntryWorker(config: { consumer: string; build: ResolvedBuildEnvironmentOptions }): boolean {
  return config.consumer === "server" && Boolean(config.build.manifest);
}

/** The modules an environment's build starts from. */
function inputs(build: ResolvedBuildEnvironmentOptions): string[] {
  const { input } = build.rollupOptions;
  return typeof input === "string" ? [input] : Object.values(input ?? {});
}

/** Writes `value` to `name` whole, as `tok` reads the file as soon as it exists. */
function writeJson(output: string, name: string, value: unknown): void {
  const file = path.join(output, name);
  fs.mkdirSync(output, { recursive: true });
  fs.writeFileSync(`${file}.tmp`, `${JSON.stringify(value, null, 2)}\n`);
  fs.renameSync(`${file}.tmp`, file);
}
