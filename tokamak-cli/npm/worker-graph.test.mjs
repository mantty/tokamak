import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import ts from "typescript";
import { tokamak } from "@tokamakdev/tok/vite";

import { createBuilder, createServer } from "vite";
import { createWorkerGraph } from "./dist/worker-graph.mjs";
import { checkTypes, project } from "./worker-graph-fixture.mjs";



test("generates editor declarations for optional native bindings and the client contract", async (t) => {
  const { root, graph } = project(t, {
    "tokamak.ts": `import { WorkerEntrypoint } from "cloudflare:workers";
    export default class Native extends WorkerEntrypoint {
      /** Gets the regional recipe.
       * @deprecated Use a recipe identifier.
       */
      recipe(longitude: number, latitude: number): { title: string } { return { title: "Soup" }; }
    }`,
  });
  await graph.types.resolve(async id => ({ id }));
  checkTypes(root, `
    // @ts-expect-error native bindings are absent on Cloudflare
    env.TOKAMAK.recipe(1, 2);
    if (env.TOKAMAK) {
      const result: Promise<{ title: string }> = env.TOKAMAK.recipe(1, 2);
    }
    const recipe: Promise<{ title: string }> = client.recipe(1, 2);
    const generatedRecipe: Promise<{ title: string }> = generated.recipe(1, 2);
    const response: Promise<Response> = service.fetch("https://app.example");
    // @ts-expect-error wrong coordinates
    client.recipe("east", "north");
    // @ts-expect-error unsupported remote-object lifetime methods
    service.dup();
  `);
});

test("infers calls through MAIN on a clean build and keeps native bindings separate", async t => {
  const sdk = JSON.stringify(fileURLToPath(import.meta.resolve("@tokamakdev/tok")));
  const events = JSON.stringify(fileURLToPath(import.meta.resolve("@tokamakdev/plugin/events")).replace(/\.ts$/, ".js"));
  const { root, config } = project(t, {
    "main.ts": `import { WorkerEntrypoint } from "cloudflare:workers";
      export default class Main extends WorkerEntrypoint<Cloudflare.Env> {
        recipe(id: number): { title: string } { return { title: String(id) }; }
        async fetch() { return Response.json(await this.env.TOKAMAK?.recipe(1)); }
      }`,
    "tokamak.ts": `import { TokamakEntrypoint } from ${sdk};
      import { defineEvent } from ${events};
      declare global { namespace Tokamak { interface Events { example: string } } }
      defineEvent("example")((_event, env, ctx) => {
        const title: Promise<{ title: string }> = env.MAIN.recipe(1);
        ctx.waitUntil(title);
        // @ts-expect-error the native worker has its own binding set
        env.TOKAMAK;
      });
      export default class Native extends TokamakEntrypoint {
        recipe(id: number) { return this.env.MAIN.recipe(id); }
        label() { return this.env.LABEL; }
        // @ts-expect-error the native worker has no main-only binding
        #mainOnly = this.env.OTHER;
      }`,
  });
  fs.appendFileSync(path.join(root, "worker-configuration.d.ts"), `
    declare namespace Cloudflare {
      interface Env { LABEL: string; DB: D1Database; OTHER: { invoke(): void } }
    }
  `);
  const builder = await createBuilder(config);
  await builder.buildApp();
  checkTypes(root, `
    declare const native: Tokamak.Env;
    const result: Promise<{ title: string }> = native.MAIN.recipe(1);
    const title: Promise<{ title: string }> = client.recipe(1);
    const label: Promise<string> = client.label();
    const variable: string = native.LABEL;
    const database: D1Database = native.DB;
    // @ts-expect-error MAIN is only a native binding
    env.MAIN;
    // @ts-expect-error native code does not inherit unrelated main bindings
    native.OTHER;
    // @ts-expect-error the native worker does not call itself through TOKAMAK
    native.TOKAMAK;
    // @ts-expect-error incorrect main method argument
    native.MAIN.recipe("one");
    // @ts-expect-error service methods are asynchronous
    const sync: { title: string } = native.MAIN.recipe(1);
    // @ts-expect-error remote-object APIs are outside the contract
    native.MAIN.dup();
  `);
});

test("generates the same inferred client contract for a clean web build", async t => {
  const sdk = JSON.stringify(fileURLToPath(import.meta.resolve("@tokamakdev/tok")));
  const { root } = project(t, {
    "main.ts": `import { WorkerEntrypoint } from "cloudflare:workers";
      import client from "virtual:tokamak/client";
      export default class Main extends WorkerEntrypoint {
        recipe(id: number): { title: string } { return { title: String(id) }; }
        fetch() { return Response.json(Object.keys(client)); }
      }`,
    "tokamak.ts": `import { TokamakEntrypoint } from ${sdk};
      export default class Native extends TokamakEntrypoint {
        recipe(id: number) { return this.env.MAIN.recipe(id); }
      }`,
  });
  fs.rmSync(path.join(root, ".tokamak"), { recursive: true });
  fs.rmSync(path.join(root, "tokamak-env.d.ts"), { force: true });
  const builder = await createBuilder({
    root, configFile: false, logLevel: "silent",
    plugins: tokamak({}, { inspectorPort: false, remoteBindings: false, persistState: false }),
  });
  await builder.buildApp();
  checkTypes(root, `
    const recipe: Promise<{ title: string }> = client.recipe(1);
    // @ts-expect-error incorrect native method argument
    client.recipe("one");
  `);
  const deploy = JSON.parse(fs.readFileSync(path.join(root, ".wrangler/deploy/config.json"), "utf8"));
  assert.deepEqual(deploy.auxiliaryWorkers ?? [], []);
});

test("preserves existing user declarations when their filename conflicts with generated types", async (t) => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "tokamak-type-conflict-"));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const file = path.join(root, "tokamak-env.d.ts");
  const source = "declare const application: string;\n";
  fs.writeFileSync(file, source);
  const graph = createWorkerGraph(root, undefined, path.join(root, ".tokamak"));
  await assert.rejects(graph.types.resolve(async id => ({ id })), /reserved for generated Tokamak types/);
  assert.equal(fs.readFileSync(file, "utf8"), source);
});

test("keeps editor types valid when an entrypoint is added, moved, or its build output is cleared", async (t) => {
  const { root } = project(t, {
    "tokamak.ts": `import { WorkerEntrypoint } from "cloudflare:workers";
    export default class Native extends WorkerEntrypoint { echo(value: string): string { return value; } }`,
  });
  const output = path.join(root, ".tokamak");
  await createWorkerGraph(root, undefined, output).types.resolve(async id => ({ id }));
  checkTypes(root, `
    // @ts-expect-error the entrypoint has no operations yet
    client.echo("value");
  `);
  await createWorkerGraph(root, path.join(root, "src/tokamak.ts"), output).types.resolve(async id => ({ id }));
  const usage = 'const value: Promise<string> = client.echo("value");';
  checkTypes(root, usage);
  fs.renameSync(path.join(root, "src/tokamak.ts"), path.join(root, "src/native.ts"));
  await createWorkerGraph(root, path.join(root, "src/native.ts"), output).types.resolve(async id => ({ id }));
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
  const server = await createServer({ ...config, server: { host: "127.0.0.1", port: 0, watch: { usePolling: true } } });
  t.after(() => server.close());
  await server.listen();
  const response = await fetch(server.resolvedUrls.local[0]);
  assert.ok(response.ok, await response.clone().text());
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
        const value = method === "value" ? typeof env.TOKAMAK.value : await env.TOKAMAK[method]("argument");
        return Response.json({ value });
      } catch (error) { return Response.json({ error: error.message }, { status: 400 }); }
    }
  };`,
  });
  const server = await createServer({ ...config, server: { host: "127.0.0.1", port: 0, watch: { usePolling: true } } });
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
  assert.deepEqual(await (await get("value")).json(), { value: "undefined" });
  for (const method of ["event", "field"]) {
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


test("rejects an unsupported main service signature during generation", async (t) => {
  const { config } = project(t, {
    "main.ts": `import { WorkerEntrypoint } from "cloudflare:workers";
      export default class Main extends WorkerEntrypoint {
        fetch() { return new Response("main"); }
        onChange(callback: () => void): void {}
      }`,
  });
  const builder = await createBuilder(config);
  await assert.rejects(builder.buildApp(), /Unsupported RPC type at onChange.callback/);
});

test("named Tokamak entrypoints cannot take names the generated worker uses", (t) => {
  for (const name of ["TokamakEvents", "TOKAMAK_LOCATION"]) {
    assert.throws(() => project(t, {
      "tokamak.ts": `import { WorkerEntrypoint } from "cloudflare:workers";
        export default class Native extends WorkerEntrypoint {}
        export class ${name} extends WorkerEntrypoint {}`,
    }), /reserves/);
  }
});
