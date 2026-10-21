import { cloudflare, type PluginConfig } from "@cloudflare/vite-plugin";
import type { Plugin } from "vite";

import { createNativeWorker, type TokamakOptions } from "./native-worker.mjs";
import { createClientPlugin } from "./client.mjs";
export type { TokamakOptions } from "./native-worker.mjs";

/** Cloudflare's Vite integration, with a separate native worker when run by tok. */
export function tokamak(options: TokamakOptions = {}, cloudflareOptions: PluginConfig = {}): Plugin[] {
  const output = process.env.TOKAMAK_VITE_OUTPUT;
  if (output && cloudflareOptions.auxiliaryWorkers?.length) {
    throw new Error("Tokamak hosts the main and native workers; additional auxiliary workers are not supported.");
  }
  const native = output ? createNativeWorker(options, output) : undefined;
  const client = createClientPlugin(options.module, !!output, native ? () => native.graph().types : undefined);
  return [...native ? [native.plugin] : [], client, ...cloudflare({
    ...cloudflareOptions,
    config(worker) {
      const custom = typeof cloudflareOptions.config === "function"
        ? cloudflareOptions.config(worker)
        : cloudflareOptions.config;
      const configured = {
        ...worker, ...custom,
        services: [...custom?.services ?? [], ...worker.services ?? []],
      };
      client.configure(configured);
      if (!native) return custom;
      const graph = native.graph();
      graph.configure(configured);
      // Cloudflare merges returned service entries with those already in the config.
      return { ...custom, main: configured.main, services: [...custom?.services ?? [], ...graph.services(graph.names().main)] };
    },
    auxiliaryWorkers: native ? [{
      viteEnvironment: { name: "tokamak" },
      config: (worker, context) => native.graph().native(worker, context),
    }] : cloudflareOptions.auxiliaryWorkers,
  })];
}
