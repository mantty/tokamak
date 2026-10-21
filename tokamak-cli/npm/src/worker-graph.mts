import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import type { PluginConfig } from "@cloudflare/vite-plugin";
import { normalizePath, type EnvironmentOptions, type Plugin } from "vite";

import type { Availability } from "@tokamakdev/plugin/capabilities";
import type { DeviceRelay } from "./native.mjs";
import type { PluginContract, WorkerContract } from "./worker-contract.mjs";
import { WorkerTypes, type NativeService, type WorkerConfiguration } from "./worker-types.mjs";

type ServiceConfiguration = Pick<WorkerConfiguration, "name" | "topLevelName" | "services" | "main" | "configPath" | "vars" | "kv_namespaces" | "d1_databases" | "r2_buckets">;
type ServiceBindings = NonNullable<ServiceConfiguration["services"]>;
type Auxiliary = NonNullable<PluginConfig["auxiliaryWorkers"]>[number];
type AuxiliaryCustomizer = Extract<Extract<Auxiliary, { config: unknown }>["config"], (...args: never[]) => unknown>;
export interface WorkerGraph {
  /** Connects development invocations to a device through `relay`. */
  relay(relay: DeviceRelay): void;
  /** Each plugin operation's native platforms, by plugin ID. */
  availability(): Availability;
  /** Configures the main Worker `config`, returning the generated service bindings it adds. */
  configure(config: ServiceConfiguration): ServiceBindings;
  /** The generated service bindings of the main Worker configuration `config`, once it binds none of their names. */
  serviceBindings(config: ServiceConfiguration): ServiceBindings;
  native: AuxiliaryCustomizer;
  names(): { main: string; native: string; app: string };
  types: WorkerTypes;
  cloudflare: PluginConfig;
  plugin: Plugin & {
    config: Extract<Plugin["config"], (...args: never[]) => unknown>;
    configEnvironment: {
      order: "post";
      handler(name: string, config: EnvironmentOptions): EnvironmentOptions;
    };
    buildStart: Extract<Plugin["buildStart"], (...args: never[]) => unknown>;
    hotUpdate: Extract<Plugin["hotUpdate"], (...args: never[]) => unknown>;
    resolveId: Extract<Plugin["resolveId"], (...args: never[]) => unknown>;
    load: Extract<Plugin["load"], (...args: never[]) => unknown>;
  };
}

const NATIVE_CONNECTION = "virtual:tokamak/native-connection";
const NATIVE_CALLS = normalizePath(fileURLToPath(new URL("./native.mjs", import.meta.url)));
/** The two Workers' roles: the app's main Worker and the Tokamak worker. */
const ROLES = ["main", "native"] as const;

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
  /** The named entrypoints the main Worker's bindings were configured with. */
  let configuredEntrypoints: string;
  const entrypoints = () => Object.keys(contracts.named).join();
  const signatures = new Map<string, string>();
  const development = normalizePath(fileURLToPath(new URL("./rpc/binding.mjs", import.meta.resolve("@tokamakdev/plugin/events"))));
  let relay: DeviceRelay | undefined;
  const serviceModule = (role: string) => `virtual:tokamak/service/${role}`;
  const bindingsModule = (role: string) => `\0tokamak:bindings:${role}`;
  const nativeContracts = (): Record<string, WorkerContract> =>
    Object.fromEntries(contracts.services().map(service => [service.entrypoint, service.contract]));
  const sources = () => [...contracts.native.files, ...Object.values(contracts.main).flatMap(value => value.files),
    ...contracts.plugins.flatMap(plugin => plugin.files)];
  const services = (name: string) => contracts.services().map(({ binding, entrypoint }) =>
    entrypoint === "default" ? { binding, service: workerName(name) } : { binding, service: workerName(name), entrypoint });
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
    const source = serviceEntrypoint(entry, contracts.services());
    writeChanged(generated, `${source}import { eventEntrypoint } from ${JSON.stringify(events)};
export const TokamakEvents = eventEntrypoint(Worker, nativeContext);\n`);
    writeChanged(path.join(output, "methods.json"), `${JSON.stringify(contracts.native.methods)}\n`);
  }
  /** The name of the main Worker configuration `config`, which binds none of the generated names. */
  const checkedName = (config: ServiceConfiguration): string => {
    if (!config.name) throw new Error("Tokamak requires a name in the main Worker's configuration.");
    const reserved = new Set(["MAIN", "TOKAMAK_RUNTIME", ...services(config.name).map(service => service.binding)]);
    const collision = bindingNames(config).find(name => reserved.has(name));
    if (collision) {
      throw new Error(`${collision} is reserved for a generated native worker binding.`);
    }
    return config.name;
  };
  const configure = (config: ServiceConfiguration): ServiceBindings => {
    mainName = checkedName(config);
    nativeName = workerName(mainName);
    appName = config.topLevelName ?? mainName;
    contracts.configure(config);
    if (dev) config.main = serviceModule("main");
    const generated = services(mainName);
    config.services = [...config.services ?? [], ...generated];
    mainServices = config.services;
    configuredEntrypoints = entrypoints();
    return generated;
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
    relay(configured) { relay = configured; },
    availability: () => availability(contracts.plugins),
    configure,
    serviceBindings: config => services(checkedName(config)),
    native,
    names: () => ({ main: mainName, native: nativeName, app: appName }),
    types: contracts,
    cloudflare: {
      config: config => { configure(config); },
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
      // After Cloudflare's plugin, which points each Worker's dependency scan
      // at its main. The scan cannot read a development service module, so it
      // reads the app's module behind it, and finds that module's dependencies
      // at startup rather than reloading the Worker when they are first
      // imported.
      configEnvironment: {
        order: "post",
        handler(name, config) {
          const served = new Map([[serviceModule("main"), contracts.mainEntry], [serviceModule("native"), entry]]);
          const entries = [config.optimizeDeps?.entries ?? []].flat()
            .map(scanned => served.get(scanned))
            .filter(module => module !== undefined)
            .map(normalizePath);
          // The main Worker's entry is the app, not a dependency, and the
          // plugin SDK keeps state that a dependency bundle would copy.
          const exclude = contracts.mainEntry ? [contracts.mainEntry] : [];
          if (name === "tokamak") exclude.push("@tokamakdev/plugin");
          return { optimizeDeps: { entries, exclude } };
        },
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
        if (entrypoints() !== configuredEntrypoints) {
          this.environment.logger.warnOnce("Restart tok dev after adding, removing, or renaming a Tokamak entrypoint.");
        }
        if (!modules.length && signatures.get(role) === signature(role)) return;
        this.environment.moduleGraph.invalidateAll();
        this.environment.hot.send({ type: "full-reload" });
        return [];
      },
      resolveId(id) {
        if ([NATIVE_CONNECTION, serviceModule("main"), serviceModule("native")].includes(id)) return `\0${id}`;
        if (id.startsWith("\0tokamak:") || id.startsWith("\0virtual:tokamak/")) return id;
      },
      async load(id) {
        if (id === `\0${NATIVE_CONNECTION}`) return nativeConnectionModule(dev ? relay : undefined);
        const role = ROLES.find(role => id === bindingsModule(role) || id === `\0${serviceModule(role)}`);
        if (!role) return;
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

/**
 * The module whose `connect(ctx)` gives an invocation its native connection: the device relay
 * during development, otherwise none, so the context uses the connection the host attached.
 */
function nativeConnectionModule(relay: DeviceRelay | undefined): string {
  if (!relay) return "export const connect = () => undefined;";
  return `import { connectDevice } from ${JSON.stringify(NATIVE_CALLS)};
export const connect = ctx => connectDevice(${JSON.stringify(relay)}, ctx);`;
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

function serviceEntrypoint(entry: string | undefined, services: NativeService[]): string {
  const sdk = normalizePath(fileURLToPath(import.meta.resolve("@tokamakdev/plugin/worker")));
  const rpc = normalizePath(fileURLToPath(new URL("./rpc/endpoint.mjs", import.meta.resolve("@tokamakdev/plugin/events"))));
  const plugins = services.flatMap(service => service.plugin ? [service.plugin] : []);
  const imports = services.map((service, index) => {
    if (service.plugin) {
      const { api } = service.plugin.definition;
      return `import { ${JSON.stringify(api.export)} as Service${index} } from ${JSON.stringify(api.module)};`;
    }
    if (!entry) return `import { TokamakEntrypoint as Service${index} } from ${JSON.stringify(sdk)};`;
    const name = service.entrypoint === "default" ? "default" : JSON.stringify(service.entrypoint);
    return `import { ${name} as Service${index} } from ${JSON.stringify(normalizePath(entry))};`;
  });
  const table = services.map((service, index) => `  { route: ${JSON.stringify(service.route)}, methods: ${JSON.stringify(service.contract.methods)}, create: ${service.plugin
    ? `ctx => Service${index}(nativeContext(ctx))`
    : `(ctx, env) => new Service${index}(nativeContext(ctx), env)`} },`);
  const methods = (methods: string[], instance: string) => methods.map(method =>
    `async [${JSON.stringify(method)}](...args) { return this.${instance}[${JSON.stringify(method)}](...args); }`).join("\n  ");
  const named = services.slice(1).map((service, index) => `
export class ${service.entrypoint} extends WorkerEntrypoint {
  #api;
  get #instance() { return this.#api ??= services[${index + 1}].create(this.ctx, this.env); }
  fetch(request) {
    ${service.plugin
      ? `return rpcResponse(request, ctx => services[${index + 1}].create(ctx, this.env), services[${index + 1}].methods, this.ctx);`
      : `if (typeof this.#instance.fetch !== "function") throw new TypeError("The ${service.entrypoint} entrypoint has no fetch handler");
    return this.#instance.fetch(request);`}
  }
  ${methods(service.contract.methods, "#instance")}
}
`);
  return `import { WorkerEntrypoint } from "cloudflare:workers";
import { rpcResponse } from ${JSON.stringify(rpc)};
import { capabilitiesModule, createNativeContext } from ${JSON.stringify(sdk)};
import { connect } from ${JSON.stringify(NATIVE_CONNECTION)};
${imports.join("\n")}

const availability = ${JSON.stringify(availability(plugins))};
const nativeContext = ctx => createNativeContext(ctx, connect(ctx), availability);
const services = [
${table.join("\n")}
];
const Worker = Service0;

export default class extends WorkerEntrypoint {
  #worker;
  get #instance() { return this.#worker ??= services[0].create(this.ctx, this.env); }
  fetch(request) {
    const { pathname } = new URL(request.url);
    if (pathname === "/_tokamak/capabilities.js") return capabilitiesModule(nativeContext(this.ctx));
    const service = services.find(service => service.route === pathname);
    if (service) return rpcResponse(request, ctx => service.create(ctx, this.env), service.methods, this.ctx);
    if (typeof this.#instance.fetch !== "function") throw new TypeError("The Tokamak worker has no fetch handler");
    return this.#instance.fetch(request);
  }
  ${methods(services[0]?.contract.methods ?? [], "#instance")}
}
${named.join("")}
`;
}

/** Writes `source` to `file` unless it already holds it, so watchers see only changes. */
export function writeChanged(file: string, source: string): void {
  if (fs.existsSync(file) && fs.readFileSync(file, "utf8") === source) return;
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, source);
}
