import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { cloudflare } from "@cloudflare/vite-plugin";
import { createWorkerGraph } from "./dist/worker-graph.mjs";
import { temporaryProject, workerTypes } from "./worker-fixtures.mjs";

export function project(t, api = `
  export function widgets(ctx: TokamakContext) {
    return {
      echo(value: string): string { return value; },
      connected(): boolean { return typeof ctx.waitUntil === "function"; },
    };
  }
`) {
  const context = JSON.stringify(fileURLToPath(import.meta.resolve("@tokamakdev/plugin")));
  const root = temporaryProject(t, "tokamak-plugin-services-", {
    "package.json": JSON.stringify({
      type: "module", dependencies: { "example-widgets": "1.0.0" },
    }),
    "packages/example-widgets/package.json": JSON.stringify({
      name: "example-widgets", type: "module", exports: { ".": "./src/index.ts" },
    }),
    "packages/example-widgets/tokamak-plugin.json": JSON.stringify({
      schemaVersion: 2, id: "widgets", api: { module: "./src/index.ts", export: "widgets" },
    }),
    "packages/example-widgets/src/index.ts": `import type { TokamakContext } from ${context};\n${api}`,
    "worker-configuration.d.ts": workerTypes,
    "tsconfig.json": JSON.stringify({ compilerOptions: {
      strict: true, target: "ES2023", module: "ESNext", moduleResolution: "Bundler",
    } }),
    "main.ts": `import { WorkerEntrypoint } from "cloudflare:workers";
    export default class Main extends WorkerEntrypoint<Cloudflare.Env> {
      async fetch() {
        return Response.json({
          value: await this.env.TOKAMAK_WIDGETS!.echo("hello"),
          connected: await this.env.TOKAMAK_WIDGETS!.connected(),
        });
      }
    }`,
    "wrangler.json": JSON.stringify({
      name: "plugin-services", main: "main.ts", compatibility_date: "2026-09-01",
    }),
  });
  const dependency = path.join(root, "packages", "example-widgets");
  fs.mkdirSync(path.join(root, "node_modules"));
  fs.symlinkSync(dependency, path.join(root, "node_modules/example-widgets"), "dir");
  const graph = createWorkerGraph(root, undefined, path.join(root, ".tokamak"));
  const config = {
    root, configFile: false, logLevel: "silent",
    server: { host: "127.0.0.1", port: 0, watch: { usePolling: true } },
    plugins: [graph.plugin, cloudflare({
      ...graph.cloudflare, inspectorPort: false, remoteBindings: false, persistState: false,
    })],
  };
  return { root, dependency, graph, config };
}
