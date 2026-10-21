import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

import { project } from "./plugin-service-fixture.mjs";
import { readOutput, serve } from "./vite-fixtures.mjs";

test("workspace API references reload; changing installed plugins requires restarting tok dev", async t => {
  const { root, dependency } = project(t);
  const app = { root, output: path.join(root, ".tokamak/vite") };
  fs.writeFileSync(path.join(root, "main.ts"), `export default {
    async fetch(_request, env) {
      const plugin = env.TOKAMAK_WIDGETS;
      return Response.json({
        bindings: Object.keys(env).filter(name => name.startsWith("TOKAMAK_")).sort(),
        value: await plugin?.echo("hello"),
      });
    }
  };`);
  await serve(app, async server => {
    const failures = [];
    const logError = server.config.logger.error;
    server.config.logger.error = (message, options) => { failures.push(message); logError(message, options); };
    let restarted;
    const restart = server.restart.bind(server);
    server.restart = (...args) => restarted = restart(...args);
    const { url } = readOutput(app, "server.json");
    assert.deepEqual(await (await fetch(url)).json(), { bindings: ["TOKAMAK_WIDGETS"], value: "hello" });
    const context = JSON.stringify(fileURLToPath(import.meta.resolve("@tokamakdev/plugin")));
    fs.writeFileSync(path.join(dependency, "src/other.ts"), `import type { TokamakContext } from ${context};
      export function gadgets(ctx: TokamakContext) { return { echo(value: string): string { return "updated " + value; } }; }`);
    fs.writeFileSync(path.join(dependency, "tokamak-plugin.json"), JSON.stringify({
      schemaVersion: 2, id: "widgets", api: { module: "./src/other.ts", export: "gadgets" },
    }));
    let result;
    for (const deadline = Date.now() + 10_000; Date.now() < deadline;) {
      // Restart briefly closes the listening socket.
      const response = await fetch(url).catch(() => undefined);
      if (response?.ok) result = await response.json();
      if (result?.value === "updated hello" && restarted) break;
      await new Promise(resolve => setTimeout(resolve, 25));
    }
    assert.deepEqual(result, { bindings: ["TOKAMAK_WIDGETS"], value: "updated hello" }, failures.join("\n"));
    await restarted;

    // Vite creates a new logger when it restarts.
    server.config.logger.error = message => failures.push(message);
    fs.writeFileSync(path.join(root, "package.json"), JSON.stringify({ type: "module", dependencies: {} }));
    for (const deadline = Date.now() + 10_000; Date.now() < deadline;) {
      if (failures.some(message => message.includes("Restart tok dev"))) break;
      await new Promise(resolve => setTimeout(resolve, 25));
    }
    assert.match(failures.join("\n"), /Restart tok dev after installing, removing, or renaming/);
  });
});
