import { randomUUID } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import cloudflare, { type Options as CloudflareOptions } from "@astrojs/cloudflare";
import type { AstroIntegration } from "astro";
import { sessionDrivers } from "astro/config";
import { normalizePath, type Plugin, type ViteDevServer } from "vite";
import { experimental_readRawConfig, unstable_readConfig, type Unstable_RawConfig } from "wrangler";

import { createNativeWorker, rejectAuxiliaryWorkers, type TokamakOptions } from "./native-worker.mjs";
import { createClientPlugin } from "./client.mjs";
import { writeChanged } from "./worker-graph.mjs";
import type { BindingConfiguration } from "./worker-types.mjs";

/** The Worker entry Astro's Cloudflare adapter builds when the configuration names none. */
const ASTRO_SERVER = "@astrojs/cloudflare/entrypoints/server";

/** Astro's Cloudflare adapter, with Tokamak's native worker and development relay. */
export function tokamak(options: TokamakOptions = {}, cloudflareOptions: CloudflareOptions = {}): AstroIntegration {
  const output = process.env.TOKAMAK_VITE_OUTPUT;
  const client = createClientPlugin(options.module, !!output, output ? () => native.graph().types : undefined);
  if (!output) {
    const adapter = cloudflare(cloudflareOptions);
    let sessionBinding: string | undefined;
    return {
      ...adapter,
      hooks: {
        ...adapter.hooks,
        async "astro:config:setup"(context) {
          const types: Plugin = {
            name: "tokamak:astro-types",
            config(vite) {
              const root = path.resolve(vite.root ?? process.cwd());
              const source = configuration(root, cloudflareOptions.configPath, false);
              const selected: BindingConfiguration = source ? readWorker(source).selected : { main: ASTRO_SERVER };
              client.configure({
                ...selected,
                // Type inspection needs the server source, not a client-optimized module.
                main: selected.main === ASTRO_SERVER ? fileURLToPath(import.meta.resolve(ASTRO_SERVER)) : selected.main,
                kv_namespaces: [...selected.kv_namespaces ?? [], ...sessionBinding ? [{ binding: sessionBinding }] : []],
              });
            },
          };
          context.updateConfig({ vite: { plugins: [client, types] } });
          await adapter.hooks["astro:config:setup"]?.(context);
        },
        async "astro:config:done"(context) {
          await adapter.hooks["astro:config:done"]?.(context);
          const driver = context.config.session ? context.config.session.driver : undefined;
          const entry = typeof driver === "string" ? driver : driver?.entrypoint.toString();
          if (entry === "cloudflareKVBinding" || entry?.endsWith("cloudflare-kv-binding") ||
            entry === sessionDrivers.cloudflareKVBinding().entrypoint) {
            sessionBinding = cloudflareOptions.sessionKVBindingName ?? "SESSION";
          }
        },
      },
    };
  }
  rejectAuxiliaryWorkers(cloudflareOptions);
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
    const { raw, selected } = readWorker(source);
    const appMain = selected.main;
    graph.configure(selected);
    // In development the graph serves main through its own module, which every environment uses.
    const servedMain = selected.main === appMain ? undefined : selected.main;
    for (const [env, config] of [[undefined, raw], ...Object.entries(raw.env ?? {})] as const) {
      const resolved = unstable_readConfig({ config: source, env }, { hideWarnings: true });
      config.services = [...config.services ?? [], ...graph.serviceBindings(resolved)];
      if (servedMain) config.main = servedMain;
    }
    // Keeping the generated configuration beside its source preserves relative paths and .dev.vars.
    writeChanged(generatedPath, `${JSON.stringify(raw, null, 2)}\n`);
    process.off("exit", cleanup);
    process.once("exit", cleanup);
  });
  const adapter = cloudflare({
    ...cloudflareOptions,
    configPath: generated,
    auxiliaryWorkers: [native.auxiliaryWorker],
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

/** The Wrangler configuration at `source`: as written, and resolved for `CLOUDFLARE_ENV` with Astro's server as its default main. */
function readWorker(source: string): { raw: Unstable_RawConfig; selected: ReturnType<typeof unstable_readConfig> } {
  const raw = experimental_readRawConfig({ config: source }, { useRedirectIfAvailable: false }).rawConfig;
  const env = process.env.CLOUDFLARE_ENV;
  const selected = unstable_readConfig({ config: source, env }, { hideWarnings: true });
  selected.main = (env ? raw.env?.[env]?.main : undefined) ?? raw.main ?? ASTRO_SERVER;
  return { raw, selected };
}

function configuration(root: string, configured: string | undefined): string;
function configuration(root: string, configured: string | undefined, required: false): string | undefined;
function configuration(root: string, configured: string | undefined, required = true): string | undefined {
  if (configured) return path.resolve(root, configured);
  const file = ["wrangler.jsonc", "wrangler.json", "wrangler.toml"]
    .map(name => path.join(root, name)).find(file => fs.existsSync(file));
  if (!file && required) throw new Error("Tokamak requires a Wrangler configuration in the Astro project root.");
  return file;
}
