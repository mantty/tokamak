import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import type { PluginConfig } from "@cloudflare/vite-plugin";
import { normalizePath, type Plugin } from "vite";

import { readServiceContracts, readWorkerContract, type WorkerContract } from "./worker-contract.mjs";
import { serviceTypes, writeEnvironmentTypes } from "./service-types.mjs";

type MainCustomizer = Extract<NonNullable<PluginConfig["config"]>, (...args: never[]) => unknown>;
type ServiceConfiguration = Pick<Parameters<MainCustomizer>[0], "name" | "topLevelName" | "services" | "main" | "configPath">;
type Auxiliary = NonNullable<PluginConfig["auxiliaryWorkers"]>[number];
type AuxiliaryCustomizer = Extract<Extract<Auxiliary, { config: unknown }>["config"], (...args: never[]) => unknown>;
export interface WorkerGraph {
  generated: string;
  configure(config: ServiceConfiguration): void;
  native: AuxiliaryCustomizer;
  names(): { main: string; native: string; app: string };
  contract(): WorkerContract;
  cloudflare: PluginConfig;
  plugin: Plugin & {
    config: Extract<Plugin["config"], (...args: never[]) => unknown>;
    configEnvironment(): { optimizeDeps: { exclude: string[] } };
    buildStart: Extract<Plugin["buildStart"], (...args: never[]) => unknown>;
    hotUpdate: Extract<Plugin["hotUpdate"], (...args: never[]) => unknown>;
    resolveId: Extract<Plugin["resolveId"], (...args: never[]) => unknown>;
    load: Extract<Plugin["load"], (...args: never[]) => unknown>;
  };
}

/** Configure the two bundles through Cloudflare's public auxiliary-worker API. */
export function createWorkerGraph(root: string, entry: string | undefined, output: string): WorkerGraph {
  const generated = path.join(output, "worker.mjs");
  const types = path.join(root, "tokamak-env.d.ts");
  const events = normalizePath(fileURLToPath(new URL("./entrypoint.js", import.meta.resolve("@tokamakdev/plugin/events"))));
  let contract: WorkerContract;
  let mainName: string;
  let nativeName: string;
  let appName: string;
  let mainEntry: string;
  let mainModule: string;
  let mainEnvironment: string;
  let dev = false;
  let mainServices: ServiceConfiguration["services"];
  let mainContracts: Record<string, WorkerContract> = {};
  const signatures = new Map<string, string>();
  const development = normalizePath(fileURLToPath(new URL("./rpc/binding.mjs", import.meta.resolve("@tokamakdev/plugin/events"))));
  const serviceModule = (role: string) => `virtual:tokamak/service/${role}`;
  const bindingsModule = (role: string) => `\0tokamak:bindings:${role}`;
  async function resolveMain(resolve: (id: string) => Promise<{ id: string } | null>): Promise<void> {
    if (mainModule) return;
    const resolved = await resolve(mainEntry);
    const source = resolved?.id.split(/[?#]/, 1)[0];
    if (!source || !fs.existsSync(source)) {
      throw new Error(`Cannot resolve the main Worker's service contract: ${mainEntry} (${resolved?.id ?? "unresolved"}).`);
    }
    mainContracts = readServiceContracts(source, root);
    mainModule = normalizePath(source);
  }
  function bindings(role: string): Record<string, string[]> {
    const services = role === "main" ? mainServices : [{ binding: "MAIN", service: mainName }];
    const result: Record<string, string[]> = {};
    for (const service of services ?? []) {
      const contracts = service.service === mainName ? mainContracts
        : service.service === nativeName ? { default: contract } : undefined;
      if (!contracts) continue;
      const target = contracts[service.entrypoint ?? "default"];
      if (!target) throw new Error(`Binding ${service.binding} refers to an unknown Worker entrypoint ${service.entrypoint}.`);
      result[service.binding] = target.methods;
    }
    return result;
  }
  function signature(role: string): string {
    const contracts = role === "main" ? mainContracts : { default: contract };
    return JSON.stringify({ bindings: bindings(role), methods: Object.entries(contracts).map(([name, value]) => [name, value.methods]) });
  }
  function refresh(): void {
    contract = entry ? readWorkerContract(entry, root) : { methods: [], files: [], declarations: serviceTypes("{}", []) };
    const source = serviceEntrypoint(entry, contract.methods);
    writeChanged(generated, `${source}import { eventEntrypoint } from ${JSON.stringify(events)};
export const TokamakEvents = eventEntrypoint(Worker);\n`);
    writeChanged(path.join(output, "methods.json"), `${JSON.stringify(contract.methods)}\n`);
    writeEnvironmentTypes(root, contract.declarations);
  }
  const configure = (config: ServiceConfiguration): void => {
    if (!config.name) throw new Error("Tokamak requires a name in the main Worker's configuration.");
    if (config.services?.some((binding) => binding.binding === "TOKAMAK")) {
      throw new Error("TOKAMAK is reserved for the generated native worker binding.");
    }
    mainName = config.name;
    nativeName = workerName(mainName);
    appName = config.topLevelName ?? mainName;
    if (!config.main) throw new Error("Tokamak requires a main Worker entrypoint.");
    const source = path.resolve(config.configPath ? path.dirname(config.configPath) : root, config.main);
    mainEntry = fs.existsSync(source) ? normalizePath(source) : config.main;
    if (dev) config.main = serviceModule("main");
    config.services = [...config.services ?? [], { binding: "TOKAMAK", service: nativeName }];
    mainServices = config.services;
  };
  const native: AuxiliaryCustomizer = (_config, { entryWorkerConfig }) => {
    if (!mainName) throw new Error("The main Worker must be configured before the Tokamak worker.");
    const { configPath, userConfigPath, compatibility_date, compatibility_flags, vars, kv_namespaces, d1_databases, r2_buckets } = entryWorkerConfig;
    return {
      configPath,
      userConfigPath,
      name: nativeName,
      main: dev ? serviceModule("native") : generated,
      compatibility_date,
      compatibility_flags,
      vars,
      kv_namespaces,
      d1_databases,
      r2_buckets,
      services: [{ binding: "MAIN", service: mainName }],
    };
  };
  refresh();
  return {
    generated: normalizePath(generated),
    configure,
    native,
    names: () => ({ main: mainName, native: nativeName, app: appName }),
    contract: () => contract,
    cloudflare: {
      config: configure,
      auxiliaryWorkers: [{
        viteEnvironment: { name: "tokamak" },
        config: native,
      }],
    },
    plugin: {
      name: "tokamak:worker-graph",
      config(_config, environment) {
        dev = environment.command === "serve";
      },
      configEnvironment() {
        return { optimizeDeps: { exclude: [mainEntry] } };
      },
      async buildStart() {
        if (this.environment.name !== "tokamak") return;
        refresh();
        await resolveMain(id => this.resolve(id, undefined, { skipSelf: true }));
        for (const file of [...contract.files, ...Object.values(mainContracts).flatMap(value => value.files)]) this.addWatchFile(file);
      },
      hotUpdate({ file, modules }) {
        if (file === normalizePath(types)) return;
        const role = this.environment.name === "tokamak" ? "native"
          : this.environment.name === mainEnvironment ? "main" : undefined;
        if (!role) return;
        const files = [...contract.files, ...Object.values(mainContracts).flatMap(value => value.files)];
        if (!files.includes(file) && (this.environment.name !== "tokamak" || !modules.length)) return;
        refresh();
        if (mainModule) mainContracts = readServiceContracts(mainModule, root);
        if (!modules.length && signatures.get(role) === signature(role)) return;
        this.environment.moduleGraph.invalidateAll();
        this.environment.hot.send({ type: "full-reload" });
        return [];
      },
      async resolveId(id) {
        if (id.startsWith("\0tokamak:") || id.startsWith("\0virtual:tokamak/")) return id;
        const role = id === serviceModule("native") ? "native" : id === serviceModule("main") ? "main" : undefined;
        if (!role) return;
        if (role === "main") mainEnvironment = this.environment.name;
        await resolveMain(source => this.resolve(source, undefined, { skipSelf: true }));
        return `\0${serviceModule(role)}`;
      },
      load(id) {
        const role = id.endsWith(":main") || id.endsWith("/main") ? "main" : "native";
        if (id === bindingsModule(role)) {
          return `import { env } from "cloudflare:workers";
import { installBindings } from ${JSON.stringify(development)};
installBindings(env, ${JSON.stringify(bindings(role))});`;
        }
        if (id !== `\0${serviceModule(role)}`) return;
        signatures.set(role, signature(role));
        const source = JSON.stringify(role === "main" ? mainModule : normalizePath(generated));
        const contracts = role === "main" ? mainContracts : { default: contract };
        const exports = Object.entries(contracts).map(([name, value], index) =>
          `const Service${index} = developmentEntrypoint(module[${JSON.stringify(name)}], ${JSON.stringify(value.methods)}, ${JSON.stringify(bindings(role))});
export { Service${index} as ${JSON.stringify(name)} };`);
        const hostEvents = role === "native"
          ? `export const TokamakEvents = developmentEvents(module.TokamakEvents, ${JSON.stringify(bindings(role))});`
          : "";
        return `import ${JSON.stringify(bindingsModule(role))};
import * as module from ${source};
export * from ${source};
import { developmentEntrypoint, developmentEvents } from ${JSON.stringify(development)};
${exports.join("\n")}
${hostEvents}`;
      },
    } satisfies Plugin,
  };
}

export function workerName(main: string): string {
  const name = `${main}-tokamak`;
  if (name.length <= 63) return name;
  return `${main.slice(0, 46)}-${createHash("sha256").update(main).digest("hex").slice(0, 8)}-tokamak`;
}

function serviceEntrypoint(entry: string | undefined, methods: string[]): string {
  const sdk = normalizePath(fileURLToPath(import.meta.resolve("@tokamakdev/plugin/worker")));
  const rpc = normalizePath(fileURLToPath(new URL("./rpc/endpoint.mjs", import.meta.resolve("@tokamakdev/plugin/events"))));
  const base = `import { WorkerEntrypoint } from "cloudflare:workers";
import { rpcResponse } from ${JSON.stringify(rpc)};
import { createNativeContext${entry ? "" : ", TokamakEntrypoint as Worker"} } from ${JSON.stringify(sdk)};
${entry ? `import Worker from ${JSON.stringify(normalizePath(entry))};` : ""}
`;
  const operations = methods.map((method) =>
    `async [${JSON.stringify(method)}](...args) { return this.#instance[${JSON.stringify(method)}](...args); }`);
  return `${base}
export default class extends WorkerEntrypoint {
  #worker;
  get #instance() { return this.#worker ??= new Worker(createNativeContext(this.ctx), this.env); }
  fetch(request) {
    if (new URL(request.url).pathname === "/_tokamak/app") {
      return rpcResponse(request, ctx => new Worker(createNativeContext(ctx), this.env), ${JSON.stringify(methods)}, this.ctx);
    }
    if (typeof this.#instance.fetch !== "function") throw new TypeError("The Tokamak worker has no fetch handler");
    return this.#instance.fetch(request);
  }
  ${operations.join("\n  ")}
}
`;
}

function writeChanged(file: string, source: string): void {
  if (fs.existsSync(file) && fs.readFileSync(file, "utf8") === source) return;
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, source);
}
