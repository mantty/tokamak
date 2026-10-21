import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

import { cloudflare } from "@cloudflare/vite-plugin";
import ts from "typescript";
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

/** Typechecks `source` against the generated client, service and environment types of the project at `root`. */
export function checkTypes(root, source) {
  const consumer = path.join(root, "src/typecheck.mts");
  fs.writeFileSync(consumer, `import generated, { type TokamakClient, type TokamakService } from "virtual:tokamak/client";
    declare const client: TokamakClient;
    declare const service: TokamakService;
    declare const env: Cloudflare.Env;
    ${source}`);
  const config = ts.readConfigFile(path.join(root, "tsconfig.json"), ts.sys.readFile);
  const parsed = ts.parseJsonConfigFileContent(config.config, ts.sys, root);
  const program = ts.createProgram([
    consumer, path.join(root, "tokamak-env.d.ts"), path.join(root, "worker-configuration.d.ts"),
  ], {
    ...parsed.options, target: ts.ScriptTarget.ES2023, module: ts.ModuleKind.NodeNext,
    moduleResolution: ts.ModuleResolutionKind.NodeNext,
    strict: true, noEmit: true, types: [], lib: ["lib.es2023.d.ts"],
  });
  assert.deepEqual(ts.getPreEmitDiagnostics(program).map((diagnostic) =>
    ts.flattenDiagnosticMessageText(diagnostic.messageText, "\n")), []);
}
