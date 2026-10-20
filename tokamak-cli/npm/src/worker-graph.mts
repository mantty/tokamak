import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";

import type { PluginConfig } from "@cloudflare/vite-plugin";
import { normalizePath, type Plugin } from "vite";

import { readWorkerContract, type WorkerContract } from "./worker-contract.mjs";

/** Generated worker configuration and the Vite hook that refreshes its contract. */
export interface WorkerGraph {
  cloudflare: PluginConfig;
  plugin: Plugin;
  contract(): WorkerContract;
}

/** Configure the two bundles through Cloudflare's public auxiliary-worker API. */
export function createWorkerGraph(root: string, entry: string, output: string): WorkerGraph {
  const generated = path.join(output, "worker.mjs");
  let contract: WorkerContract;
  let mainName: string;
  let nativeName: string;
  function refresh(): void {
    contract = readWorkerContract(entry, root);
    writeChanged(generated, `export { default } from ${JSON.stringify(normalizePath(entry))};\n`);
    writeChanged(path.join(output, "methods.json"), `${JSON.stringify(contract.methods)}\n`);
  }
  refresh();
  return {
    contract: () => contract,
    cloudflare: {
      config(config) {
        if (!config.name) throw new Error("Tokamak requires a name in the main Worker's configuration.");
        if (config.services?.some((binding) => binding.binding === "TOKAMAK")) {
          throw new Error("TOKAMAK is reserved for the generated native worker binding.");
        }
        mainName = config.name;
        nativeName = workerName(mainName);
        config.services = [...config.services ?? [], { binding: "TOKAMAK", service: nativeName }];
      },
      auxiliaryWorkers: [{
        viteEnvironment: { name: "tokamak" },
        config(_config, { entryWorkerConfig }) {
          if (!mainName) throw new Error("The main Worker must be configured before the Tokamak worker.");
          const { compatibility_date, compatibility_flags, vars, kv_namespaces, d1_databases, r2_buckets } = entryWorkerConfig;
          return {
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
        },
      }],
    },
    plugin: {
      name: "tokamak:worker-graph",
      buildStart() {
        refresh();
        for (const file of contract.files) this.addWatchFile(file);
      },
      hotUpdate() {
        if (this.environment.name === "tokamak") refresh();
      },
    },
  };
}

function workerName(main: string): string {
  const name = `${main}-tokamak`;
  if (name.length <= 63) return name;
  return `${main.slice(0, 46)}-${createHash("sha256").update(main).digest("hex").slice(0, 8)}-tokamak`;
}

function writeChanged(file: string, source: string): void {
  if (fs.existsSync(file) && fs.readFileSync(file, "utf8") === source) return;
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, source);
}
