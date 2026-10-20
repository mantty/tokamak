import fs from "node:fs";
import path from "node:path";
import type { PluginConfig } from "@cloudflare/vite-plugin";

import { readWorkerContracts, type PluginContract, type WorkerContract } from "./worker-contract.mjs";
import { serviceTypes, writeEnvironmentTypes } from "./service-types.mjs";
import { discoverPlugins, type PluginDefinition } from "./plugin-manifest.mjs";

type MainCustomizer = Extract<NonNullable<PluginConfig["config"]>, (...args: never[]) => unknown>;
export type WorkerConfiguration = Parameters<MainCustomizer>[0];
export type BindingConfiguration = Partial<Pick<WorkerConfiguration, "main" | "configPath" | "vars" | "kv_namespaces" | "d1_databases" | "r2_buckets">>;

/** Shared contracts for the two entrypoints and their generated callers. */
export class WorkerTypes {
  native: WorkerContract = { methods: [], files: [], declarations: serviceTypes("{}", []) };
  main: Record<string, WorkerContract> = {};
  plugins: PluginContract[] = [];
  definitions: PluginDefinition[];
  mainEntry?: string;
  mainModule?: string;
  #bindings: string[] = [];
  #ready = false;
  #change?: { file: string; timestamp: number };

  constructor(readonly root: string, readonly entry: string | undefined) {
    this.definitions = discoverPlugins(root);
  }

  configure(config: BindingConfiguration): void {
    if (!config.main) throw new Error("Tokamak requires a main Worker entrypoint.");
    const source = path.resolve(config.configPath ? path.dirname(config.configPath) : this.root, config.main);
    this.mainEntry = fs.existsSync(source) ? source : config.main;
    this.mainModule = undefined;
    this.#ready = false;
    this.bindings(config);
  }

  bindings(config: BindingConfiguration): void {
    this.#ready = false;
    this.#bindings = [...new Set([
      ...Object.keys(config.vars ?? {}),
      ...[...config.kv_namespaces ?? [], ...config.d1_databases ?? [], ...config.r2_buckets ?? []].map(value => value.binding),
    ])].sort();
  }

  async resolve(resolve: (id: string) => Promise<{ id: string } | null>): Promise<void> {
    if (this.#ready) return;
    if (this.mainEntry && !this.mainModule) {
      const resolved = await resolve(this.mainEntry);
      const source = resolved?.id.split(/[?#]/, 1)[0];
      if (!source || !fs.existsSync(source)) {
        throw new Error(`Cannot resolve the main Worker's service contract: ${this.mainEntry} (${resolved?.id ?? "unresolved"}).`);
      }
      this.mainModule = source.split(path.sep).join("/");
    }
    this.refresh();
  }

  discover(): void {
    const definitions = discoverPlugins(this.root);
    if (definitions.length !== this.definitions.length || definitions.some((plugin, index) =>
      plugin.id !== this.definitions[index].id || plugin.package !== this.definitions[index].package)) {
      throw new Error("Restart tok dev after installing, removing, or renaming a Tokamak plugin.");
    }
    this.definitions = definitions;
    this.#read(false);
  }

  get metadata(): string[] {
    return [path.join(this.root, "package.json"), ...this.definitions.map(plugin => plugin.manifest)]
      .map(file => file.split(path.sep).join("/"));
  }

  refresh(change?: { file: string; timestamp: number }): void {
    if (this.mainEntry && !this.mainModule) return;
    if (this.#ready && change && change.file === this.#change?.file && change.timestamp === this.#change.timestamp) return;
    this.#ready = false;
    this.discover();
    // Declare both APIs before checking signatures inferred from cross-worker calls.
    writeEnvironmentTypes(this.root, this.native.declarations,
      this.main.default?.declarations ?? serviceTypes("{}", [], "Main"), this.#bindings, this.plugins);
    this.#read(true);
    this.#ready = true;
    this.#change = change;
  }

  #read(validate: boolean): void {
    const contracts = readWorkerContracts(this.entry, this.mainModule, this.root, validate, this.definitions);
    this.native = contracts.native;
    this.main = contracts.main;
    this.plugins = contracts.plugins;
  }
}
