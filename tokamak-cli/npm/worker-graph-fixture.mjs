import fs from "node:fs";
import os from "node:os";
import path from "node:path";

import { cloudflare } from "@cloudflare/vite-plugin";
import { createWorkerGraph } from "./dist/worker-graph.mjs";
import { workerTypes } from "./worker-fixtures.mjs";

export function project(t, sources = {}) {
  const root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), "tokamak-graph-")));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  fs.mkdirSync(path.join(root, "src"));
  fs.writeFileSync(path.join(root, "worker-configuration.d.ts"), workerTypes);
  fs.writeFileSync(path.join(root, "wrangler.json"), JSON.stringify({
    name: "graph-app", main: "src/main.ts", compatibility_date: "2026-09-01", vars: { LABEL: "configured" },
    d1_databases: [{ binding: "DB", database_id: "00000000-0000-0000-0000-000000000000", database_name: "app", migrations_dir: "db/migrations" }]
  }));
  fs.mkdirSync(path.join(root, "db/migrations"), { recursive: true });
  fs.writeFileSync(path.join(root, "db/migrations/001.sql"), "CREATE TABLE entries (value TEXT);");
  fs.writeFileSync(path.join(root, "src/main.ts"), `import { WorkerEntrypoint } from "cloudflare:workers";
    export default class Main extends WorkerEntrypoint {
      async fetch() { return Response.json(await this.env.TOKAMAK.describe()); }
      label() { return "main-only"; }
    }`);
  fs.writeFileSync(path.join(root, "src/shared.ts"), 'export const shared = "shared-source";');
  fs.writeFileSync(path.join(root, "src/tokamak.ts"), `import { WorkerEntrypoint } from "cloudflare:workers";
    import { shared } from "@app/shared";
    export default class Native extends WorkerEntrypoint {
      async describe(): Promise<{ label: string; main: string; shared: string }> {
        return { label: this.env.LABEL, main: await this.env.MAIN.label(), shared };
      }
    }`);
  fs.writeFileSync(path.join(root, "tsconfig.json"), JSON.stringify({ compilerOptions: {
    baseUrl: ".", paths: { "@app/*": ["src/*"] }
  } }));
  for (const [file, source] of Object.entries(sources)) fs.writeFileSync(path.join(root, "src", file), source);
  const graph = createWorkerGraph(root, path.join(root, "src/tokamak.ts"), path.join(root, ".tokamak"));
  return { root, graph, config: {
    root, configFile: false, logLevel: "silent",
    resolve: { alias: { "@app": path.join(root, "src") } },
    plugins: [graph.plugin, cloudflare({ ...graph.cloudflare, inspectorPort: false, remoteBindings: false, persistState: false })]
  } };
}
