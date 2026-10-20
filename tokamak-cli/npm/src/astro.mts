import { randomUUID } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import cloudflare, { type Options as CloudflareOptions } from "@astrojs/cloudflare";
import type { AstroIntegration } from "astro";
import { normalizePath, type Plugin, type ViteDevServer } from "vite";
import { experimental_readRawConfig, unstable_readConfig, type Unstable_RawConfig } from "wrangler";

import { createNativeWorker, type TokamakOptions } from "./native-worker.mjs";
import { createClientPlugin } from "./client.mjs";
import { workerName } from "./worker-graph.mjs";

/** Astro's Cloudflare adapter, with Tokamak's native worker and development relay. */
export function tokamak(options: TokamakOptions = {}, cloudflareOptions: CloudflareOptions = {}): AstroIntegration {
  const output = process.env.TOKAMAK_VITE_OUTPUT;
  const client = createClientPlugin(options.module, !!output);
  if (!output) {
    const adapter = cloudflare(cloudflareOptions);
    return {
      ...adapter,
      hooks: {
        ...adapter.hooks,
        async "astro:config:setup"(context) {
          context.updateConfig({ vite: { plugins: [client] } });
          await adapter.hooks["astro:config:setup"]?.(context);
        },
      },
    };
  }
  if (cloudflareOptions.auxiliaryWorkers?.length) {
    throw new Error("Tokamak hosts the main and native workers; additional auxiliary workers are not supported.");
  }
  const generated = path.join(path.dirname(cloudflareOptions.configPath ?? "."), `.tokamak.${randomUUID()}.json`);
  let generatedPath: string | undefined;
  const cleanup = () => {
    if (generatedPath) fs.rmSync(generatedPath, { force: true });
    process.off("exit", cleanup);
  };
  const servers = new Set<NonNullable<ViteDevServer["httpServer"]>>();
  const lifecycle: Plugin = {
    name: "tokamak:astro-config",
    configureServer({ httpServer }) {
      if (!httpServer) return;
      servers.add(httpServer);
      httpServer.once("close", () => {
        servers.delete(httpServer);
        if (!servers.size) cleanup();
      });
    },
    buildEnd(error) {
      if (error && this.environment.mode === "build") cleanup();
    },
  };
  const native = createNativeWorker(options, output, (root, graph) => {
    const source = configuration(root, cloudflareOptions.configPath);
    generatedPath = path.resolve(root, generated);
    const rawConfig: Unstable_RawConfig = experimental_readRawConfig(
      { config: source }, { useRedirectIfAvailable: false },
    ).rawConfig;
    const environment = process.env.CLOUDFLARE_ENV;
    const selected = unstable_readConfig({ config: source, env: environment }, { hideWarnings: true });
    selected.main = (environment ? rawConfig.env?.[environment]?.main : undefined)
      ?? rawConfig.main ?? "@astrojs/cloudflare/entrypoints/server";
    const main = selected.main;
    graph.configure(selected);
    const environments = [[undefined, rawConfig], ...Object.entries(rawConfig.env ?? {})] as const;
    for (const [env, config] of environments) {
      const resolved = unstable_readConfig({ config: source, env }, { hideWarnings: true });
      if (!resolved.name) throw new Error("Tokamak requires a name in the main Worker's configuration.");
      if (config.services?.some(binding => binding.binding === "TOKAMAK")) {
        throw new Error("TOKAMAK is reserved for the generated native worker binding.");
      }
      config.services = [...config.services ?? [], { binding: "TOKAMAK", service: workerName(resolved.name) }];
      if (selected.main !== main) config.main = selected.main;
    }
    // Keeping the generated configuration beside its source preserves relative paths and .dev.vars.
    const content = `${JSON.stringify(rawConfig, null, 2)}\n`;
    if (!fs.existsSync(generatedPath) || fs.readFileSync(generatedPath, "utf8") !== content) {
      fs.writeFileSync(generatedPath, content);
    }
    process.off("exit", cleanup);
    process.once("exit", cleanup);
  });
  const adapter = cloudflare({
    ...cloudflareOptions,
    configPath: generated,
    auxiliaryWorkers: [{
      viteEnvironment: { name: "tokamak" },
      config: (worker, context) => native.graph().native(worker, context),
    }],
  });
  return {
    ...adapter,
    name: "@tokamakdev/tok/astro",
    hooks: {
      ...adapter.hooks,
      async "astro:config:setup"(context) {
        const source = configuration(fileURLToPath(context.config.root), cloudflareOptions.configPath);
        context.addWatchFile(pathToFileURL(source));
        context.updateConfig({
          vite: {
            server: { fs: { deny: [normalizePath(source)] } },
            plugins: [native.plugin, client, lifecycle],
          },
        });
        await adapter.hooks["astro:config:setup"]?.(context);
      },
      async "astro:build:done"(context) {
        try {
          await adapter.hooks["astro:build:done"]?.(context);
        } finally {
          cleanup();
        }
      },
    },
  };
}

function configuration(root: string, configured: string | undefined): string {
  if (configured) return path.resolve(root, configured);
  const file = ["wrangler.jsonc", "wrangler.json", "wrangler.toml"]
    .map(name => path.join(root, name)).find(file => fs.existsSync(file));
  if (!file) throw new Error("Tokamak requires a Wrangler configuration in the Astro project root.");
  return file;
}
