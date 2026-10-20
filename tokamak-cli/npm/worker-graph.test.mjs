import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";
import ts from "typescript";

import { cloudflare } from "@cloudflare/vite-plugin";
import { createBuilder, createServer } from "vite";
import { createWorkerGraph } from "./dist/worker-graph.mjs";
import { workerTypes } from "./worker-fixtures.mjs";

function project(t, sources = {}) {
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

function checkTypes(root, source) {
  const consumer = path.join(root, "src/typecheck.mts");
  fs.writeFileSync(consumer, `import type { TokamakClient, TokamakService } from "virtual:tokamak/client";
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

test("generates editor declarations for optional native bindings and the client contract", (t) => {
  const { root } = project(t, {
    "tokamak.ts": `import { WorkerEntrypoint } from "cloudflare:workers";
    export default class Native extends WorkerEntrypoint {
      /** Gets the regional recipe.
       * @deprecated Use a recipe identifier.
       */
      recipe(longitude: number, latitude: number): { title: string } { return { title: "Soup" }; }
    }`,
  });
  checkTypes(root, `
    // @ts-expect-error native bindings are absent on Cloudflare
    env.TOKAMAK.recipe(1, 2);
    if (env.TOKAMAK) {
      const result: Promise<{ title: string }> = env.TOKAMAK.recipe(1, 2);
    }
    const recipe: Promise<{ title: string }> = client.recipe(1, 2);
    const response: Promise<Response> = service.fetch("https://app.example");
    // @ts-expect-error wrong coordinates
    client.recipe("east", "north");
    // @ts-expect-error unsupported remote-object lifetime methods
    service.dup();
  `);
});

test("preserves existing user declarations when their filename conflicts with generated types", (t) => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "tokamak-type-conflict-"));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const file = path.join(root, "tokamak-env.d.ts");
  const source = "declare const application: string;\n";
  fs.writeFileSync(file, source);
  assert.throws(() => createWorkerGraph(root, undefined, path.join(root, ".tokamak")), /reserved for generated Tokamak types/);
  assert.equal(fs.readFileSync(file, "utf8"), source);
});

test("keeps editor types valid when an entrypoint is added, moved, or its build output is cleared", (t) => {
  const { root } = project(t, {
    "tokamak.ts": `import { WorkerEntrypoint } from "cloudflare:workers";
    export default class Native extends WorkerEntrypoint { echo(value: string): string { return value; } }`,
  });
  const output = path.join(root, ".tokamak");
  createWorkerGraph(root, undefined, output);
  checkTypes(root, `
    // @ts-expect-error the entrypoint has no operations yet
    client.echo("value");
  `);
  createWorkerGraph(root, path.join(root, "src/tokamak.ts"), output);
  const usage = 'const value: Promise<string> = client.echo("value");';
  checkTypes(root, usage);
  fs.renameSync(path.join(root, "src/tokamak.ts"), path.join(root, "src/native.ts"));
  createWorkerGraph(root, path.join(root, "src/native.ts"), output);
  fs.rmSync(output, { recursive: true, force: true });
  checkTypes(root, usage);
});

test("builds two separate bundles with generated service bindings and project aliases", async (t) => {
  const { root, graph, config } = project(t);
  const builder = await createBuilder(config);
  await builder.buildApp();
  const pointer = path.join(root, ".wrangler/deploy/config.json");
  const deploy = JSON.parse(fs.readFileSync(pointer, "utf8"));
  const read = (file) => JSON.parse(fs.readFileSync(path.resolve(path.dirname(pointer), file), "utf8"));
  const main = read(deploy.configPath);
  const native = read(deploy.auxiliaryWorkers[0].configPath);
  assert.deepEqual(main.services, [{ binding: "TOKAMAK", service: "graph-app-tokamak" }]);
  assert.deepEqual(native.services, [{ binding: "MAIN", service: "graph-app" }]);
  assert.deepEqual(native.vars, { LABEL: "configured" });
  const migrations = path.resolve(path.dirname(pointer), deploy.auxiliaryWorkers[0].configPath, "..", native.d1_databases[0].migrations_dir);
  assert.equal(fs.readFileSync(path.join(migrations, "001.sql"), "utf8"), "CREATE TABLE entries (value TEXT);");
  assert.deepEqual(graph.contract().methods, ["describe"]);
  const mainSource = fs.readFileSync(path.resolve(path.dirname(pointer), deploy.configPath, "..", main.main), "utf8");
  const nativeSource = fs.readFileSync(path.resolve(path.dirname(pointer), deploy.auxiliaryWorkers[0].configPath, "..", native.main), "utf8");
  assert.match(mainSource, /main-only/);
  assert.doesNotMatch(mainSource, /shared-source/);
  assert.match(nativeSource, /shared-source/);
  assert.doesNotMatch(nativeSource, /main-only/);
});

test("serves bidirectional RPC and reloads imported native code through Vite", async (t) => {
  const { root, config } = project(t);
  const server = await createServer({ ...config, server: { port: 0, watch: { usePolling: true } } });
  t.after(() => server.close());
  await server.listen();
  const response = await fetch(server.resolvedUrls.local[0]);
  assert.deepEqual(await response.json(), { label: "configured", main: "main-only", shared: "shared-source" });
  fs.writeFileSync(path.join(root, "src/shared.ts"), 'export const shared = "updated-source";');
  let result;
  for (const deadline = Date.now() + 10_000; Date.now() < deadline;) {
    result = await (await fetch(server.resolvedUrls.local[0])).json();
    if (result.shared === "updated-source") break;
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
  assert.deepEqual(result, { label: "configured", main: "main-only", shared: "updated-source" });
});

test("rejects an unsupported public signature before building a bundle", async (t) => {
  const { root, config } = project(t);
  fs.writeFileSync(path.join(root, "src/tokamak.ts"), `import { WorkerEntrypoint } from "cloudflare:workers";
  export default class Native extends WorkerEntrypoint {
    listen(callback: () => void) {}
  }`);
  const builder = await createBuilder(config);
  await assert.rejects(builder.buildApp(), /Unsupported RPC type at listen.callback/);
});

test("service bindings expose validated methods and fetch while keeping events and properties local", async (t) => {
  const { root, config } = project(t, {
    "tokamak.ts": `import { WorkerEntrypoint } from "cloudflare:workers";
    class Base extends WorkerEntrypoint<{ LABEL: string }> {
      inherited(value: string): string { return this.env.LABEL + ":" + value; }
    }
    export default class Native extends Base {
      #prefix = "private";
      field = () => "local field";
      get value() { return "local property"; }
      event(): string { return "host only"; }
      describe(): string { return this.#prefix; }
      fetch(request: Request): Response { return new Response(request.url); }
    }`,
    "main.ts": `export default {
    async fetch(request, env) {
      const method = new URL(request.url).pathname.slice(1);
      try {
        if (method === "fetch") return env.TOKAMAK.fetch("https://service.example/native");
        const value = method === "value" ? await env.TOKAMAK.value : await env.TOKAMAK[method]("argument");
        return Response.json({ value });
      } catch (error) { return Response.json({ error: error.message }, { status: 400 }); }
    }
  };`,
  });
  const server = await createServer({ ...config, server: { port: 0, watch: { usePolling: true } } });
  t.after(() => server.close());
  await server.listen();
  const get = (method) => fetch(server.resolvedUrls.local[0] + method);
  assert.deepEqual(await (await get("describe")).json(), { value: "private" });
  checkTypes(root, `
    const before: Promise<string> = client.describe();
    const inherited: Promise<string> = client.inherited("argument");
  `);
  assert.deepEqual(await (await get("inherited")).json(), { value: "configured:argument" });
  assert.equal(await (await get("fetch")).text(), "https://service.example/native");
  for (const method of ["event", "field", "value"]) {
    const response = await get(method);
    assert.equal(response.status, 400, `${method} must not be exposed by the service binding`);
    assert.match((await response.json()).error, /does not implement|not a function|not.*exposed/i);
  }
  const entry = path.join(root, "src/tokamak.ts");
  fs.writeFileSync(entry, fs.readFileSync(entry, "utf8").replace(
    'describe(): string { return this.#prefix; }',
    'added(): string { return this.#prefix + ":updated"; }',
  ));
  let updated;
  for (const deadline = Date.now() + 10_000; Date.now() < deadline;) {
    updated = await get("added");
    if (updated.ok) break;
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
  assert.deepEqual(await updated.json(), { value: "private:updated" });
  assert.equal((await get("describe")).status, 400);
  checkTypes(root, `
    const after: Promise<string> = client.added();
    // @ts-expect-error the removed method is absent from refreshed declarations
    client.describe();
  `);
});
