import { cloudflare, type PluginConfig } from "@cloudflare/vite-plugin";
import type { Plugin } from "vite";

import { createNativeWorker, type TokamakOptions } from "./native-worker.mjs";
export type { TokamakOptions } from "./native-worker.mjs";

/** Cloudflare's Vite integration, with a separate native worker when run by tok. */
export function tokamak(options: TokamakOptions = {}, cloudflareOptions: PluginConfig = {}): Plugin[] {
  const output = process.env.TOKAMAK_VITE_OUTPUT;
  if (!output) return cloudflare(cloudflareOptions);
  if (cloudflareOptions.auxiliaryWorkers?.length) {
    throw new Error("Tokamak hosts the main and native workers; additional auxiliary workers are not supported.");
  }
  const native = createNativeWorker(options, output);
  return [native.plugin, ...cloudflare({
    ...cloudflareOptions,
    config(worker) {
      const custom = typeof cloudflareOptions.config === "function"
        ? cloudflareOptions.config(worker)
        : cloudflareOptions.config;
      const configured = {
        ...worker, ...custom,
        services: [...custom?.services ?? [], ...worker.services ?? []],
      };
      const graph = native.graph();
      graph.configure(configured);
      // Cloudflare merges returned service entries with those already in the config.
      return { ...custom, services: [...custom?.services ?? [], { binding: "TOKAMAK", service: graph.names().native }] };
    },
    auxiliaryWorkers: [{
      viteEnvironment: { name: "tokamak" },
      config: (worker, context) => native.graph().native(worker, context),
    }],
  })];
}
