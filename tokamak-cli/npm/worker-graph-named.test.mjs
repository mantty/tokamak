import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { test } from "node:test";

import { createServer } from "vite";
import { checkTypes, project } from "./worker-graph-fixture.mjs";

// Cloudflare's Vite plugin keeps each Worker's exports for the process, so a
// project serving other exports than worker-graph.test.mjs's runs on its own.
test("named Tokamak entrypoints get generated bindings, clients and routes", async (t) => {
  const { root, graph, config } = project(t, {
    "main.ts": `import { WorkerEntrypoint } from "cloudflare:workers";
      export default class Main extends WorkerEntrypoint<Cloudflare.Env> {
        async fetch() { return Response.json({ pulled: await this.env.TOKAMAK_DATA_SYNC!.pull(2) }); }
        label() { return "main-only"; }
      }`,
    "tokamak.ts": `import { WorkerEntrypoint } from "cloudflare:workers";
      export default class Native extends WorkerEntrypoint {
        describe(): string { return "default"; }
      }
      export class DataSync extends WorkerEntrypoint {
        pull(count: number): number[] { return Array.from({ length: count }, (_, index) => index); }
      }`,
  });
  await graph.types.resolve(async id => ({ id }));
  checkTypes(root, `
    import sync from "virtual:tokamak/client/DataSync";
    const pulled: Promise<number[]> = sync.pull(1);
    const capability: boolean = sync.capabilities.pull;
    if (env.TOKAMAK_DATA_SYNC) {
      const remote: Promise<number[]> = env.TOKAMAK_DATA_SYNC.pull(1);
    }
    // @ts-expect-error the default entrypoint's methods stay on TOKAMAK
    sync.describe();
  `);
  assert.deepEqual(graph.serviceBindings({ name: "graph-app" }), [
    { binding: "TOKAMAK", service: "graph-app-tokamak" },
    { binding: "TOKAMAK_DATA_SYNC", service: "graph-app-tokamak", entrypoint: "DataSync" },
  ]);
  const server = await createServer({ ...config, server: { host: "127.0.0.1", port: 0, watch: { usePolling: true } } });
  t.after(() => server.close());
  await server.listen();
  const response = await fetch(server.resolvedUrls.local[0]);
  assert.ok(response.ok, await response.clone().text());
  assert.deepEqual(await response.json(), { pulled: [0, 1] });

  // The main Worker's bindings are set when the server starts.
  const warnings = [];
  server.config.logger.warnOnce = message => warnings.push(message);
  const entry = path.join(root, "src/tokamak.ts");
  fs.appendFileSync(entry, `
      export class Archive extends WorkerEntrypoint {
        count(): number { return 0; }
      }`);
  for (const deadline = Date.now() + 10_000; Date.now() < deadline;) {
    if (warnings.some(message => message.includes("Restart tok dev"))) break;
    await new Promise(resolve => setTimeout(resolve, 25));
  }
  assert.match(warnings.join("\n"), /Restart tok dev after adding, removing, or renaming a Tokamak entrypoint/);
});
