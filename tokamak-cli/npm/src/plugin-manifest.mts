import fs from "node:fs";
import { createRequire } from "node:module";
import path from "node:path";

import type { NativePlatform } from "@tokamakdev/plugin/capabilities";

export type { NativePlatform };

export interface ModuleExport {
  module: string;
  export: string;
}

/** Where an operation runs, and what it needs there. */
export interface Operation {
  /** The native platforms that implement it, with any minimum OS version. */
  platforms: { name: NativePlatform; minimum?: string }[];
  /** The native platforms where it needs the app in the foreground, to show UI. */
  foreground: NativePlatform[];
}

export interface PluginDefinition {
  id: string;
  package: string;
  manifest: string;
  api: ModuleExport;
  web?: ModuleExport;
  binding: string;
  client: string;
  /** The native platforms the plugin builds for. */
  platforms: NativePlatform[];
  /** Each operation the manifest describes; others run on every platform the plugin builds for. */
  operations: Record<string, Operation>;
}

const nativePlatforms: readonly NativePlatform[] = ["ios", "macos", "android", "windows", "linux"];

const reservedIds = ["tokamak", "android", "ios", "macos", "windows", "name", "identifier", "icon", "version",
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
    const platforms = nativePlatforms.filter(platform => data.platforms?.[platform] !== undefined);
    plugins.set(data.id, {
      id: data.id, package: name, manifest, api,
      web: data.web === undefined ? undefined : moduleExport(directory, data.web, "web"),
      binding: `TOKAMAK_${data.id.toUpperCase().replaceAll("-", "_")}`,
      client: `${name}/client`,
      platforms,
      operations: operations(manifest, data.operations ?? {}, platforms),
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

/** The manifest's `operations`, each on platforms the plugin builds for. */
function operations(manifest: string, value: unknown, built: NativePlatform[]): Record<string, Operation> {
  const fail = (message: string): never => { throw new Error(`${manifest}: ${message}`); };
  const platform = (name: string, field: string): NativePlatform =>
    built.includes(name as NativePlatform) ? name as NativePlatform : fail(`${field} names ${name}, which the plugin does not build for.`);
  if (!isRecord(value)) return fail("operations must be an object.");
  const result: Record<string, Operation> = {};
  for (const [name, entry] of Object.entries(value)) {
    const field = `operations.${name}`;
    if (!isRecord(entry)) return fail(`${field} must be an object.`);
    const { platforms, foreground = false, ...unknown } = entry;
    if (Object.keys(unknown).length) fail(`${field} has unknown keys: ${Object.keys(unknown).join(", ")}.`);
    if (platforms !== undefined && (!isRecord(platforms) ||
      !Object.values(platforms).every(version => typeof version === "string" || version === true))) {
      fail(`${field}.platforms must map platforms to a minimum OS version, or true for the app's minimum.`);
    }
    if (typeof foreground !== "boolean" && !(Array.isArray(foreground) && foreground.every(name => typeof name === "string"))) {
      fail(`${field}.foreground must be true or a list of platforms.`);
    }
    const implemented = platforms === undefined
      ? built.map(name => ({ name }))
      : Object.entries(platforms as Record<string, string | true>).map(([name, minimum]) =>
        ({ name: platform(name, `${field}.platforms`), minimum: minimum === true ? undefined : minimum }));
    const needs = foreground === true ? implemented.map(({ name }) => name) : foreground === false ? [] : foreground as string[];
    result[name] = { platforms: implemented, foreground: needs.map(name => platform(name, `${field}.foreground`)) };
  }
  return result;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
