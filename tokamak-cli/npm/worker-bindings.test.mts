import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { test } from "node:test";

import { createServer } from "vite";
import { localURL } from "./worker-fixtures.mts";
import { project } from "./worker-graph-fixture.mts";

/** What the main Worker's fetch handler returns: a call's value, or the error that failed it. */
interface Result {
  value?: string;
  error?: string;
  code?: string;
}

test("generated bindings enforce the RPC contract in both directions, including captured and named bindings", async (t) => {
  const { root, config } = project(t, {
    "main.ts": `import { WorkerEntrypoint, env } from "cloudflare:workers";
      const native = env.TOKAMAK;
      const recipes = env.RECIPES;
      export class Recipes extends WorkerEntrypoint { title(id: number): string { return "recipe:" + id; } }
      export default class Main extends WorkerEntrypoint {
        echo(value: string): string { return value; }
        invalid(): string { return (() => "callback") as unknown as string; }
        async fetch(request: Request): Promise<Response> {
          const method = new URL(request.url).pathname.slice(1);
          try {
            const value = method === "recipe" ? await recipes.title(3)
              : method === "argument" ? await native.echo(() => "callback")
              : method === "handlerArgument" ? await this.env.TOKAMAK.echo(() => "callback")
              : await native[method]();
            return Response.json({ value });
          } catch (error) { return Response.json({ error: error.name, code: error.code }); }
        }
      }`,
    "tokamak.ts": `import { WorkerEntrypoint, env } from "cloudflare:workers";
      const main = env.MAIN;
      export default class Native extends WorkerEntrypoint {
        echo(value: string): string { return value; }
        invalid(): string { return (() => "callback") as unknown as string; }
        fail(): never { throw Object.assign(new Error("Denied"), { code: "DENIED" }); }
        async mainArgument(): Promise<string> { return main.echo(() => "callback"); }
        async mainResult(): Promise<string> { return main.invalid(); }
      }`,
  });
  const file = path.join(root, "wrangler.json");
  const worker = JSON.parse(fs.readFileSync(file, "utf8"));
  worker.compatibility_date = "2025-01-01";
  worker.services = [{ binding: "RECIPES", service: "graph-app", entrypoint: "Recipes" }];
  fs.writeFileSync(file, JSON.stringify(worker));
  const server = await createServer({ ...config, server: { host: "127.0.0.1", port: 0, watch: { usePolling: true } } });
  t.after(() => server.close());
  await server.listen();
  const get = async (method: string): Promise<Result> => (await fetch(localURL(server) + method)).json();
  assert.deepEqual(await get("recipe"), { value: "recipe:3" });
  for (const method of ["argument", "handlerArgument", "invalid", "mainArgument", "mainResult"]) {
    assert.equal((await get(method)).error, "DataCloneError", method);
  }
  assert.deepEqual(await get("fail"), { error: "Error", code: "DENIED" });
  const entry = path.join(root, "src/main.ts");
  fs.writeFileSync(entry, fs.readFileSync(entry, "utf8")
    .replace("title(id: number)", "heading(id: number)")
    .replace("recipes.title(3)", "recipes.heading(3)")
    .replace('"recipe:"', '"updated:"'));
  let updated: Result | undefined;
  for (const deadline = Date.now() + 10_000; Date.now() < deadline;) {
    updated = await get("recipe");
    if (updated.value === "updated:3") break;
    await new Promise(resolve => setTimeout(resolve, 50));
  }
  assert.deepEqual(updated, { value: "updated:3" });
});
