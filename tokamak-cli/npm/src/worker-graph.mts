import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import type { PluginConfig } from "@cloudflare/vite-plugin";
import { normalizePath, type Plugin } from "vite";

import { readWorkerContract, type WorkerContract } from "./worker-contract.mjs";

type MainCustomizer = Extract<NonNullable<PluginConfig["config"]>, (...args: never[]) => unknown>;
type ServiceConfiguration = Pick<Parameters<MainCustomizer>[0], "name" | "topLevelName" | "services">;
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
    buildStart: Extract<Plugin["buildStart"], (...args: never[]) => unknown>;
    hotUpdate: Extract<Plugin["hotUpdate"], (...args: never[]) => unknown>;
  };
}

/** Configure the two bundles through Cloudflare's public auxiliary-worker API. */
export function createWorkerGraph(root: string, entry: string | undefined, output: string): WorkerGraph {
  const generated = path.join(output, "worker.mjs");
  const events = normalizePath(fileURLToPath(new URL("./entrypoint.js", import.meta.resolve("@tokamakdev/plugin/events"))));
  let contract: WorkerContract;
  let mainName: string;
  let nativeName: string;
  let appName: string;
  function refresh(): void {
    contract = entry ? readWorkerContract(entry, root) : { methods: [], files: [] };
    const source = serviceEntrypoint(entry, contract.methods);
    writeChanged(generated, `${source}import { eventEntrypoint } from ${JSON.stringify(events)};
export const TokamakEvents = eventEntrypoint(Worker);\n`);
    writeChanged(path.join(output, "methods.json"), `${JSON.stringify(contract.methods)}\n`);
  }
  const configure = (config: ServiceConfiguration): void => {
    if (!config.name) throw new Error("Tokamak requires a name in the main Worker's configuration.");
    if (config.services?.some((binding) => binding.binding === "TOKAMAK")) {
      throw new Error("TOKAMAK is reserved for the generated native worker binding.");
    }
    mainName = config.name;
    nativeName = workerName(mainName);
    appName = config.topLevelName ?? mainName;
    config.services = [...config.services ?? [], { binding: "TOKAMAK", service: nativeName }];
  };
  const native: AuxiliaryCustomizer = (_config, { entryWorkerConfig }) => {
    if (!mainName) throw new Error("The main Worker must be configured before the Tokamak worker.");
    const { configPath, userConfigPath, compatibility_date, compatibility_flags, vars, kv_namespaces, d1_databases, r2_buckets } = entryWorkerConfig;
    return {
      configPath,
      userConfigPath,
      name: nativeName,
      main: generated,
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
      buildStart() {
        if (this.environment.name !== "tokamak") return;
        refresh();
        for (const file of contract.files) this.addWatchFile(file);
      },
      hotUpdate({ file, modules }) {
        if (this.environment.name !== "tokamak" || (!modules.length && !contract.files.includes(file))) return;
        refresh();
        this.environment.moduleGraph.invalidateAll();
        this.environment.hot.send({ type: "full-reload" });
        return [];
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
  const base = `import { WorkerEntrypoint } from "cloudflare:workers";
import { createNativeContext${entry ? "" : ", TokamakEntrypoint as Worker"} } from ${JSON.stringify(sdk)};
${entry ? `import Worker from ${JSON.stringify(normalizePath(entry))};` : ""}
`;
  const operations = methods.map((method) =>
    `async [${JSON.stringify(method)}](...args) { return this.#worker[${JSON.stringify(method)}](...args); }`);
  return `${base}
export default class extends WorkerEntrypoint {
  #worker = new Worker(createNativeContext(this.ctx), this.env);
  fetch(request) {
    if (typeof this.#worker.fetch !== "function") throw new TypeError("The Tokamak worker has no fetch handler");
    return this.#worker.fetch(request);
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
