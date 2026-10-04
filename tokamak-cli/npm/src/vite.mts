import fs from "node:fs";
import path from "node:path";
import { normalizePath, runnerImport, type Plugin, type ResolvedConfig } from "vite";

/** Configuration files tried in order when `tok` names none. */
const CONFIG_FILES = ["src/tokamak.ts", "src/tokamak.js"];

/** The files each output directory's `config.json` was read from, once written. */
const written = new Map<string, Promise<string[]>>();

/**
 * Reports the build's tokamak configuration and development server address to
 * `tok`. Without `TOKAMAK_VITE_OUTPUT`, which `tok` sets, it adds no hooks.
 */
export function tokamak(): Plugin[] {
  const output = process.env.TOKAMAK_VITE_OUTPUT;
  if (!output) {
    return [];
  }
  let config: ResolvedConfig;
  const write = () => {
    const file = findConfigFile(config.root);
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

/** Writes `value` to `name` whole, as `tok` reads the file as soon as it exists. */
function writeJson(output: string, name: string, value: unknown): void {
  const file = path.join(output, name);
  fs.mkdirSync(output, { recursive: true });
  fs.writeFileSync(`${file}.tmp`, `${JSON.stringify(value, null, 2)}\n`);
  fs.renameSync(`${file}.tmp`, file);
}
