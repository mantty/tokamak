import fs from "node:fs";
import { createRequire } from "node:module";
import path from "node:path";

export interface ModuleExport {
  module: string;
  export: string;
}

export interface PluginDefinition {
  id: string;
  package: string;
  manifest: string;
  api: ModuleExport;
  web?: ModuleExport;
  binding: string;
  client: string;
}

const reservedIds = ["android", "ios", "macos", "windows", "name", "identifier", "icon", "version",
  "module", "build", "project", "platform", "skip", "host", "help", "vite", "session"];

/** Installed direct dependencies select the application's plugin services. */
export function discoverPlugins(root: string): PluginDefinition[] {
  const file = path.join(root, "package.json");
  if (!fs.existsSync(file)) return [];
  const app = JSON.parse(fs.readFileSync(file, "utf8"));
  const dependencies = new Set<string>(["dependencies", "devDependencies", "peerDependencies"]
    .flatMap(key => Object.keys(app[key] ?? {})));
  const require = createRequire(file);
  const plugins = new Map<string, PluginDefinition>();
  for (const name of dependencies) {
    const directory = require.resolve.paths(name)?.map(base => path.join(base, name))
      .find(candidate => fs.existsSync(path.join(candidate, "package.json")));
    if (!directory) continue;
    const candidate = path.join(directory, "tokamak-plugin.json");
    if (!fs.existsSync(candidate)) continue;
    const manifest = fs.realpathSync(candidate).split(path.sep).join("/");
    const data = JSON.parse(fs.readFileSync(manifest, "utf8"));
    // Schema version one supplies only native build metadata.
    if (data.schemaVersion === 1) continue;
    if (data.schemaVersion !== 2) throw new Error(`${manifest}: unsupported plugin schema version ${data.schemaVersion}.`);
    if (typeof data.id !== "string" || !/^[a-z][a-z0-9]*(?:-[a-z0-9]+)*$/.test(data.id) || data.id.length > 63) {
      throw new Error(`${manifest}: plugin id must be lowercase words joined by single hyphens.`);
    }
    const reserved = reservedIds.find(word => data.id === word || data.id.startsWith(`${word}-`));
    if (reserved) throw new Error(`${manifest}: plugin id must not start with '${reserved}', which Tokamak's options use.`);
    const api = moduleExport(directory, data.api, "api");
    if (plugins.has(data.id)) throw new Error(`Duplicate Tokamak plugin id ${data.id}.`);
    plugins.set(data.id, {
      id: data.id, package: name, manifest, api,
      web: data.web === undefined ? undefined : moduleExport(directory, data.web, "web"),
      binding: `TOKAMAK_${data.id.toUpperCase().replaceAll("-", "_")}`,
      client: `${name}/client`,
    });
  }
  return [...plugins.values()].sort((left, right) => left.id.localeCompare(right.id));
}

function moduleExport(root: string, reference: unknown, field: string): ModuleExport {
  const value = reference as Partial<ModuleExport> | null;
  if (!value || typeof value.module !== "string" || typeof value.export !== "string" || !value.export) {
    throw new Error(`${root}: ${field} must name a module and export.`);
  }
  const module = fs.realpathSync(path.resolve(root, value.module));
  const relative = path.relative(fs.realpathSync(root), module);
  if (relative === ".." || relative.startsWith(`..${path.sep}`) || path.isAbsolute(relative) || !fs.statSync(module).isFile()) {
    throw new Error(`${root}: ${field}.module must be a file inside the plugin package.`);
  }
  return { module: module.split(path.sep).join("/"), export: value.export };
}
