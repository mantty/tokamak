import fs from "node:fs";
import path from "node:path";
import {
  normalizePath,
  runnerImport,
  type Plugin,
  type ResolvedBuildEnvironmentOptions,
  type ResolvedConfig,
  type Rollup,
} from "vite";

/** Configuration files tried in order when `tok` names none. */
const CONFIG_FILES = ["src/tokamak.ts", "src/tokamak.js"];

/** Wrangler configuration files Cloudflare's plugin tries in order. */
const WRANGLER_FILES = ["wrangler.jsonc", "wrangler.json", "wrangler.toml"];

/** The files each output directory's `config.json` was read from, once written. */
const written = new Map<string, Promise<string[]>>();

/**
 * Reports the build's tokamak configuration, and the development server's
 * address and Worker name, to `tok`, and makes the entry Worker import the
 * configuration file. Without `TOKAMAK_VITE_OUTPUT`, which `tok` sets, it adds
 * no hooks.
 */
export function tokamak(): Plugin[] {
  const output = process.env.TOKAMAK_VITE_OUTPUT;
  if (!output) {
    return [];
  }
  let config: ResolvedConfig;
  let file: string | undefined;
  /** The module IDs of each entry environment's inputs. */
  const entryIds = new Map<string, Promise<(string | undefined)[]>>();
  /** The entry environments whose input imports the configuration file. */
  const importing = new Set<string>();
  const write = () => {
    const files = writeConfig(output, file, config);
    // A file that fails to load stays watched, so fixing it rewrites `config.json`.
    written.set(output, files.catch(() => (file ? [file] : [])));
    return files;
  };
  return [
    {
      name: "tokamak",
      async configResolved(resolved) {
        config = resolved;
        file = findConfigFile(resolved.root);
        await (written.has(output) ? written.get(output) : write());
      },
      async watchChange(id) {
        if ((await written.get(output))?.includes(id)) {
          await write();
        }
      },
      async configureServer(server) {
        const httpServer = server.httpServer;
        if (!httpServer) {
          return;
        }
        const workerName = await readWorkerName(server.config.root);
        httpServer.on("listening", () => {
          const urls = server.resolvedUrls;
          writeJson(output, "server.json", { url: urls?.local[0] ?? urls?.network[0], workerName });
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
        if (id === file) {
          return withoutConfig(this, code);
        }
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

/** The configuration file `TOKAMAK_CONFIG` names, or the first default under `root` that exists. */
function findConfigFile(root: string): string | undefined {
  const named = process.env.TOKAMAK_CONFIG;
  if (named) {
    if (!fs.existsSync(named)) {
      throw new Error(`tokamak configuration file not found: ${named}`);
    }
    return normalizePath(path.resolve(named));
  }
  const file = CONFIG_FILES.map((name) => path.resolve(root, name)).find((candidate) =>
    fs.existsSync(candidate),
  );
  return file && normalizePath(file);
}

/**
 * Writes the file's `config` export, with the file's path, to `config.json`,
 * returning the files it was read from.
 */
async function writeConfig(
  output: string,
  file: string | undefined,
  config: ResolvedConfig,
): Promise<string[]> {
  if (!file) {
    writeJson(output, "config.json", { config: {} });
    return [];
  }
  const { module, dependencies } = await runnerImport<{ config?: unknown }>(file, {
    root: config.root,
    resolve: { alias: config.resolve.alias, tsconfigPaths: config.resolve.tsconfigPaths },
  });
  writeJson(output, "config.json", { file, config: module.config ?? {} });
  return [file, ...dependencies.map(normalizePath)];
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

/** The parts of an ESTree statement `withoutConfig` reads. */
interface Statement {
  type: string;
  start: number;
  end: number;
  declaration?: {
    start: number;
    id?: { name?: string } | null;
    declarations?: { id: { name?: string } }[];
  } | null;
  specifiers?: { exported: { name?: string } }[];
}

/**
 * `code` without its `config` export: the `export const config = ...`
 * statement is blanked, or only its `export` keyword while other statements
 * mention `config`, so that every other position, and so the source map, is
 * unchanged.
 */
function withoutConfig(context: Rollup.TransformPluginContext, code: string) {
  const statements = context.parse(code).body as Statement[];
  const statement = statements.find(exportsConfig);
  if (!statement) {
    return;
  }
  const declaration = statement.declaration;
  if (declaration?.declarations?.length !== 1) {
    return context.error("declare config in its own `export const config = ...` statement");
  }
  const mentioned = statements.some((other) => other !== statement && mentions(other, "config"));
  const end = mentioned ? declaration.start : statement.end;
  const blank = code.slice(statement.start, end).replace(/[^\n]/g, " ");
  return { code: code.slice(0, statement.start) + blank + code.slice(end), map: null };
}

/** Whether `node` contains an identifier named `name`, property names included. */
function mentions(node: unknown, name: string): boolean {
  if (typeof node !== "object" || node === null) {
    return false;
  }
  const identifier = node as { type?: unknown; name?: unknown };
  if (identifier.type === "Identifier" && identifier.name === name) {
    return true;
  }
  return Object.values(node).some((child) => mentions(child, name));
}

function exportsConfig(statement: Statement): boolean {
  if (statement.type !== "ExportNamedDeclaration") {
    return false;
  }
  const declaration = statement.declaration;
  const names = [
    declaration?.id?.name,
    ...(declaration?.declarations ?? []).map((declarator) => declarator.id.name),
    ...(statement.specifiers ?? []).map((specifier) => specifier.exported.name),
  ];
  return names.includes("config");
}

/** Writes `value` to `name` whole, as `tok` reads the file as soon as it exists. */
function writeJson(output: string, name: string, value: unknown): void {
  const file = path.join(output, name);
  fs.mkdirSync(output, { recursive: true });
  fs.writeFileSync(`${file}.tmp`, `${JSON.stringify(value, null, 2)}\n`);
  fs.renameSync(`${file}.tmp`, file);
}
