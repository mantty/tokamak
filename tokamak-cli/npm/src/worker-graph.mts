import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import type { PluginConfig } from "@cloudflare/vite-plugin";
import { normalizePath, type Plugin } from "vite";

import type { Availability, NativePlatform } from "@tokamakdev/plugin/capabilities";
import type { PluginContract, WorkerContract } from "./worker-contract.mjs";
import { WorkerTypes, type WorkerConfiguration } from "./worker-types.mjs";

type ServiceConfiguration = Pick<WorkerConfiguration, "name" | "topLevelName" | "services" | "main" | "configPath" | "vars" | "kv_namespaces" | "d1_databases" | "r2_buckets">;
type Auxiliary = NonNullable<PluginConfig["auxiliaryWorkers"]>[number];
type AuxiliaryCustomizer = Extract<Extract<Auxiliary, { config: unknown }>["config"], (...args: never[]) => unknown>;
export interface WorkerGraph {
  generated: string;
  /** Connects development invocations to a device on `platform` through the relay on `port`. */
  relay(port: number, token: string, platform: NativePlatform | undefined): void;
  /** Each plugin operation's native platforms, by plugin ID. */
  availability(): Availability;
  configure(config: ServiceConfiguration): void;
  services(name: string): NonNullable<ServiceConfiguration["services"]>;
  native: AuxiliaryCustomizer;
  names(): { main: string; native: string; app: string };
  contract(): WorkerContract;
  types: WorkerTypes;
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

const NATIVE_CONNECTION = "virtual:tokamak/native-connection";
const NATIVE_CALLS = normalizePath(fileURLToPath(new URL("./native.mjs", import.meta.url)));

/** Configure the two bundles through Cloudflare's public auxiliary-worker API. */
export function createWorkerGraph(root: string, entry: string | undefined, output: string): WorkerGraph {
  const generated = path.join(output, "worker.mjs");
  const types = path.join(root, "tokamak-env.d.ts");
  const contracts = new WorkerTypes(root, entry);
  contracts.discover();
  const events = normalizePath(fileURLToPath(new URL("./entrypoint.js", import.meta.resolve("@tokamakdev/plugin/events"))));
  let mainName: string;
  let nativeName: string;
  let appName: string;
  let mainEnvironment: string;
  let dev = false;
  let mainServices: ServiceConfiguration["services"];
  const signatures = new Map<string, string>();
  const development = normalizePath(fileURLToPath(new URL("./rpc/binding.mjs", import.meta.resolve("@tokamakdev/plugin/events"))));
  let relay: Relay | undefined;
  const serviceModule = (role: string) => `virtual:tokamak/service/${role}`;
  const bindingsModule = (role: string) => `\0tokamak:bindings:${role}`;
  const nativeContracts = () => Object.fromEntries([
    ["default", contracts.native],
    ...contracts.plugins.map(plugin => [plugin.definition.binding, plugin]),
  ]) as Record<string, WorkerContract>;
  const sources = () => [...contracts.native.files, ...Object.values(contracts.main).flatMap(value => value.files),
    ...contracts.plugins.flatMap(plugin => plugin.files)];
  const services = (name: string) => [
    { binding: "TOKAMAK", service: workerName(name) },
    ...contracts.definitions.map(plugin => ({ binding: plugin.binding, service: workerName(name), entrypoint: plugin.binding })),
  ];
  function bindings(role: string): Record<string, string[]> {
    const services = role === "main" ? mainServices : [{ binding: "MAIN", service: mainName }];
    const result: Record<string, string[]> = {};
    for (const service of services ?? []) {
      const targets = service.service === mainName ? contracts.main
        : service.service === nativeName ? nativeContracts() : undefined;
      if (!targets) continue;
      const target = targets[service.entrypoint ?? "default"];
      if (!target) throw new Error(`Binding ${service.binding} refers to an unknown Worker entrypoint ${service.entrypoint}.`);
      result[service.binding] = target.methods;
    }
    return result;
  }
  function signature(role: string): string {
    const targets = role === "main" ? contracts.main : nativeContracts();
    return JSON.stringify({ bindings: bindings(role), methods: Object.entries(targets).map(([name, value]) => [name, value.methods]) });
  }
  function refresh(): void {
    const source = serviceEntrypoint(entry, contracts.native.methods, contracts.plugins);
    writeChanged(generated, `${source}import { eventEntrypoint } from ${JSON.stringify(events)};
export const TokamakEvents = eventEntrypoint(Worker, nativeContext);\n`);
    writeChanged(path.join(output, "methods.json"), `${JSON.stringify(contracts.native.methods)}\n`);
  }
  const configure = (config: ServiceConfiguration): void => {
    if (!config.name) throw new Error("Tokamak requires a name in the main Worker's configuration.");
    const reserved = new Set(["MAIN", "TOKAMAK_RUNTIME", ...services(config.name).map(service => service.binding)]);
    const collision = bindingNames(config).find(name => reserved.has(name));
    if (collision) {
      throw new Error(`${collision} is reserved for a generated native worker binding.`);
    }
    mainName = config.name;
    nativeName = workerName(mainName);
    appName = config.topLevelName ?? mainName;
    contracts.configure(config);
    if (dev) config.main = serviceModule("main");
    config.services = [...config.services ?? [], ...services(mainName)];
    mainServices = config.services;
  };
  const native: AuxiliaryCustomizer = (_config, { entryWorkerConfig }) => {
    if (!mainName) throw new Error("The main Worker must be configured before the Tokamak worker.");
    contracts.bindings(entryWorkerConfig);
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
    relay(port, token, platform) { relay = { port, token, platform }; },
    availability: () => availability(contracts.plugins),
    configure,
    services,
    native,
    names: () => ({ main: mainName, native: nativeName, app: appName }),
    contract: () => contracts.native,
    types: contracts,
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
        contracts.discover();
      },
      configEnvironment() {
        return { optimizeDeps: { exclude: contracts.mainEntry ? [contracts.mainEntry] : [] } };
      },
      async buildStart() {
        if (this.environment.name !== "tokamak") return;
        for (const file of contracts.metadata) this.addWatchFile(file);
        await contracts.resolve(id => this.resolve(id, undefined, { skipSelf: true }));
        refresh();
        for (const file of sources()) this.addWatchFile(file);
      },
      hotUpdate({ file, modules, timestamp }) {
        if (file === normalizePath(types)) return;
        if (contracts.metadata.includes(file)) return [];
        const role = this.environment.name === "tokamak" ? "native"
          : this.environment.name === mainEnvironment ? "main" : undefined;
        if (!role) return;
        if (!sources().includes(file) && (this.environment.name !== "tokamak" || !modules.length)) return;
        contracts.refresh({ file, timestamp });
        refresh();
        if (!modules.length && signatures.get(role) === signature(role)) return;
        this.environment.moduleGraph.invalidateAll();
        this.environment.hot.send({ type: "full-reload" });
        return [];
      },
      resolveId(id) {
        if (id === NATIVE_CONNECTION) return `\0${NATIVE_CONNECTION}`;
        if (id.startsWith("\0tokamak:") || id.startsWith("\0virtual:tokamak/")) return id;
        const role = id === serviceModule("native") ? "native" : id === serviceModule("main") ? "main" : undefined;
        if (!role) return;
        return `\0${serviceModule(role)}`;
      },
      async load(id) {
        if (id === `\0${NATIVE_CONNECTION}`) return nativeConnectionModule(dev ? relay : undefined);
        const role = id.endsWith(":main") || id.endsWith("/main") ? "main" : "native";
        if (id !== bindingsModule(role) && id !== `\0${serviceModule(role)}`) return;
        await contracts.resolve(source => this.resolve(source, undefined, { skipSelf: true }));
        refresh();
        if (role === "main") mainEnvironment = this.environment.name;
        if (id === bindingsModule(role)) {
          return `import { env } from "cloudflare:workers";
import { installBindings } from ${JSON.stringify(development)};
installBindings(env, ${JSON.stringify(bindings(role))});`;
        }
        signatures.set(role, signature(role));
        const source = JSON.stringify(role === "main" ? contracts.mainModule : normalizePath(generated));
        const targets = role === "main" ? contracts.main : nativeContracts();
        const exports = Object.entries(targets).map(([name, value], index) =>
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

/** The binding kinds of a Worker configuration besides variables, whose entries name bindings. */
const BINDING_KINDS = [
  "ai", "analytics_engine_datasets", "assets", "browser", "d1_databases", "dispatch_namespaces", "durable_objects",
  "hyperdrive", "images", "kv_namespaces", "mtls_certificates", "pipelines", "queues", "r2_buckets", "rate_limiting",
  "secrets_store_secrets", "send_email", "services", "vectorize", "version_metadata", "workflows",
];

/** Every binding name `config` declares, as the native packager reads them. */
function bindingNames(config: object): string[] {
  const names = Object.keys((config as { vars?: object }).vars ?? {});
  const collect = (value: unknown): void => {
    if (Array.isArray(value)) return value.forEach(collect);
    if (!value || typeof value !== "object") return;
    const entry = value as Record<string, unknown>;
    const nested = ["bindings", "producers", "consumers"].filter(key => key in entry);
    if (nested.length) return nested.forEach(key => { collect(entry[key]); });
    const name = ["binding", "name", "queue", "dataset"].map(key => entry[key]).find(name => typeof name === "string");
    if (typeof name === "string") names.push(name);
  };
  for (const kind of BINDING_KINDS) collect((config as Record<string, unknown>)[kind]);
  return names;
}

export function workerName(main: string): string {
  const name = `${main}-tokamak`;
  if (name.length <= 63) return name;
  return `${main.slice(0, 46)}-${createHash("sha256").update(main).digest("hex").slice(0, 8)}-tokamak`;
}

/** The device relay a development Worker reaches native code through. */
interface Relay {
  port: number;
  token: string;
  platform: NativePlatform | undefined;
}

/**
 * The module whose `connect(ctx)` gives an invocation its native connection: the device relay
 * during development, otherwise none, so the context uses the connection the host attached.
 */
function nativeConnectionModule(relay: Relay | undefined): string {
  if (!relay) return "export const connect = () => undefined;";
  return `import { connectDevice } from ${JSON.stringify(NATIVE_CALLS)};
export const connect = ctx => connectDevice(${String(relay.port)}, ${JSON.stringify(relay.token)}, ctx, ${JSON.stringify(relay.platform ?? null)} ?? undefined);`;
}

/** Each plugin operation's native platforms: those its manifest names, or every one the plugin builds for. */
function availability(plugins: PluginContract[]): Availability {
  return Object.fromEntries(plugins.map(({ definition, operations }) => [
    definition.id,
    Object.fromEntries(operations.map(operation => [
      operation,
      definition.operations[operation]?.platforms.map(({ name }) => name) ?? definition.platforms,
    ])),
  ]));
}

function serviceEntrypoint(entry: string | undefined, methods: string[], plugins: PluginContract[]): string {
  const sdk = normalizePath(fileURLToPath(import.meta.resolve("@tokamakdev/plugin/worker")));
  const rpc = normalizePath(fileURLToPath(new URL("./rpc/endpoint.mjs", import.meta.resolve("@tokamakdev/plugin/events"))));
  const base = `import { WorkerEntrypoint } from "cloudflare:workers";
import { rpcResponse } from ${JSON.stringify(rpc)};
import { capabilitiesModule, createNativeContext${entry ? "" : ", TokamakEntrypoint as Worker"} } from ${JSON.stringify(sdk)};
import { connect } from ${JSON.stringify(NATIVE_CONNECTION)};
${entry ? `import Worker from ${JSON.stringify(normalizePath(entry))};` : ""}
${plugins.map((plugin, index) => `import { ${JSON.stringify(plugin.definition.api.export)} as plugin${index} } from ${JSON.stringify(plugin.definition.api.module)};`).join("\n")}
`;
  const operations = methods.map((method) =>
    `async [${JSON.stringify(method)}](...args) { return this.#instance[${JSON.stringify(method)}](...args); }`);
  return `${base}
const availability = ${JSON.stringify(availability(plugins))};
const nativeContext = ctx => createNativeContext(ctx, connect(ctx), availability);

export default class extends WorkerEntrypoint {
  #worker;
  get #instance() { return this.#worker ??= new Worker(nativeContext(this.ctx), this.env); }
  fetch(request) {
    if (new URL(request.url).pathname === "/_tokamak/capabilities.js") return capabilitiesModule(nativeContext(this.ctx));
    if (new URL(request.url).pathname === "/_tokamak/app") {
      return rpcResponse(request, ctx => new Worker(nativeContext(ctx), this.env), ${JSON.stringify(methods)}, this.ctx);
    }
    ${plugins.map((plugin, index) => `if (new URL(request.url).pathname === ${JSON.stringify(`/_tokamak/plugin/${plugin.definition.id}`)}) {
      return rpcResponse(request, ctx => plugin${index}(nativeContext(ctx)), ${JSON.stringify(plugin.methods)}, this.ctx);
    }`).join("\n    ")}
    if (typeof this.#instance.fetch !== "function") throw new TypeError("The Tokamak worker has no fetch handler");
    return this.#instance.fetch(request);
  }
  ${operations.join("\n  ")}
}
${plugins.map((plugin, index) => `
export class ${plugin.definition.binding} extends WorkerEntrypoint {
  #api;
  get #instance() { return this.#api ??= plugin${index}(nativeContext(this.ctx)); }
  fetch(request) {
    return rpcResponse(request, ctx => plugin${index}(nativeContext(ctx)), ${JSON.stringify(plugin.methods)}, this.ctx);
  }
  ${plugin.methods.map(method => `async [${JSON.stringify(method)}](...args) { return this.#instance[${JSON.stringify(method)}](...args); }`).join("\n  ")}
}
`).join("\n")}
`;
}

function writeChanged(file: string, source: string): void {
  if (fs.existsSync(file) && fs.readFileSync(file, "utf8") === source) return;
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, source);
}
