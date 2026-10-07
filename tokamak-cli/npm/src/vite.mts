import { X509Certificate } from "node:crypto";
import fs from "node:fs";
import type { ServerOptions as HttpsServerOptions } from "node:https";
import path from "node:path";
import { normalizePath, type Plugin, type ResolvedBuildEnvironmentOptions } from "vite";

import type { Config } from "./index.mjs";

/** Modules tried in order when the options name none. */
const DEFAULT_MODULES = ["src/tokamak.ts", "src/tokamak.js"];

/** Wrangler configuration files Cloudflare's plugin tries in order. */
const WRANGLER_FILES = ["wrangler.jsonc", "wrangler.json", "wrangler.toml"];

const MIDDLEWARE_MODE_ERROR =
  "Vite is running in middleware mode, so tokamak cannot find the development server's address; tok dev needs Vite's own dev server";

/**
 * Reports the app's tokamak configuration, and the development server's
 * address, HTTPS certificate and Worker name, to `tok`, and makes the entry
 * Worker import `module`. Without `TOKAMAK_VITE_OUTPUT`, which `tok` sets, it
 * adds no hooks.
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
  /** The module IDs of each entry environment's inputs. */
  const entryIds = new Map<string, Promise<(string | undefined)[]>>();
  /** The entry environments whose input imports the module. */
  const importing = new Set<string>();
  return [
    {
      name: "tokamak",
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
        const workerName = await readWorkerName(server.config.root);
        httpServer.on("listening", () => {
          const urls = server.resolvedUrls;
          writeJson(output, "server.json", {
            url: urls?.local[0] ?? urls?.network[0],
            workerName,
            certificates: endEntityCertificates(server.config.server.https),
          });
        });
      },
      async buildApp(builder) {
        const environments = Object.values(builder.environments);
        if (file && !environments.some((environment) => buildsEntryWorker(environment.config))) {
          throw new Error(`no environment builds the entry Worker to import ${file} into`);
        }
      },
    },
    {
      name: "tokamak:entry",
      applyToEnvironment: ({ config }) => file !== undefined && buildsEntryWorker(config),
      async transform(code, id) {
        const environment = this.environment;
        const ids =
          entryIds.get(environment.name) ??
          Promise.all(inputs(environment.config.build).map(async (input) => (await this.resolve(input))?.id));
        entryIds.set(environment.name, ids);
        if (!(await ids).includes(id)) {
          return;
        }
        importing.add(environment.name);
        // In Cloudflare's Worker entry, which dev and build both start from,
        // this follows the imports of its polyfills and of the app's entry.
        return { code: `${code}\nimport ${JSON.stringify(file)};\n`, map: null };
      },
      generateBundle() {
        if (!importing.has(this.environment.name)) {
          this.error(`the input of the ${this.environment.name} environment did not import ${file}`);
        }
      },
    },
  ];
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
 * The name of the Worker in the Wrangler configuration file in `root`, found
 * as Cloudflare's plugin finds it without `configPath`, without the suffix of
 * the `CLOUDFLARE_ENV` environment.
 */
async function readWorkerName(root: string): Promise<string | undefined> {
  const file = WRANGLER_FILES.map((name) => path.join(root, name)).find((candidate) => fs.existsSync(candidate));
  if (!file) {
    return undefined;
  }
  const { unstable_readConfig } = await import("wrangler");
  const worker = unstable_readConfig({ config: file, env: process.env.CLOUDFLARE_ENV }, { hideWarnings: true });
  return worker.topLevelName ?? worker.name;
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
