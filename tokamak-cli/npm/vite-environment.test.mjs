import assert from "node:assert/strict";
import { test } from "node:test";

import { project, readOutput, serve, withEnvironment } from "./vite-fixtures.mjs";

// Cloudflare's plugin keeps the named exports of the first development Worker
// in a process for later servers, so a Worker of another name, as in another
// environment, needs a process of its own.
test("reports the Worker's top-level name with the development server of an environment", async () => {
  const app = project({
    "wrangler.jsonc": JSON.stringify({
      name: "app",
      main: "src/index.ts",
      compatibility_date: "2026-09-01",
      env: { staging: {} },
    }),
  });
  await withEnvironment({ CLOUDFLARE_ENV: "staging" }, () =>
    serve(app, () => assert.equal(readOutput(app, "server.json").workerName, "app")),
  );
});
