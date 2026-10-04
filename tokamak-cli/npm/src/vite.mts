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

/** The files each output directory's `config.json` was read from, once written. */
const written = new Map<string, Promise<string[]>>();

/**
 * Reports the build's tokamak configuration and development server address to
 * `tok`, and makes the entry Worker import the configuration file. Without
 * `TOKAMAK_VITE_OUTPUT`, which `tok` sets, it adds no hooks.
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
      configureServer(server) {
        server.httpServer?.on("listening", () => {
          const urls = server.resolvedUrls;
          writeJson(output, "server.json", { url: urls?.local[0] ?? urls?.network[0] });
        });
      },
    },
    {
      name: "tokamak:entry",
      // Server environments with a manifest are built from the entry Worker:
      // Cloudflare's plugin gives one to the entry and prerender Workers, and
      // Astro to its server and prerender environments.
      applyToEnvironment: ({ config }) =>
        file !== undefined && config.consumer === "server" && Boolean(config.build.manifest),
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
    id?: { name?: string } | null;
    declarations?: { id: { name?: string } }[];
  } | null;
  specifiers?: { exported: { name?: string } }[];
}

/**
 * `code` without its `export const config = ...` statement, which is blanked
 * so that every other position, and so the source map, is unchanged.
 */
function withoutConfig(context: Rollup.TransformPluginContext, code: string) {
  const statements = context.parse(code).body as Statement[];
  const statement = statements.find(exportsConfig);
  if (!statement) {
    return;
  }
  if (statement.declaration?.declarations?.length !== 1) {
    context.error("declare config in its own `export const config = ...` statement");
  }
  const blank = code.slice(statement.start, statement.end).replace(/[^\n]/g, " ");
  return { code: code.slice(0, statement.start) + blank + code.slice(statement.end), map: null };
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
