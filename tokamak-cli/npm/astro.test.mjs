import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";
import { fileURLToPath, pathToFileURL } from "node:url";

import { build, dev } from "astro";
import { workerTypes } from "./worker-fixtures.mjs";

function project(t) {
  const root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), "tokamak-astro-")));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  for (const directory of ["src/pages", "config", "db"]) fs.mkdirSync(path.join(root, directory), { recursive: true });
  fs.symlinkSync(fileURLToPath(new URL("./node_modules", import.meta.url)), path.join(root, "node_modules"), "junction");
  fs.writeFileSync(path.join(root, "package.json"), JSON.stringify({ name: "astro-fixture", type: "module" }));
  fs.writeFileSync(path.join(root, "worker-configuration.d.ts"), workerTypes);
  const wrangler = {
    name: "astro-app", main: "@astrojs/cloudflare/entrypoints/server", compatibility_date: "2026-09-01",
    compatibility_flags: ["nodejs_compat"], assets: { directory: "../dist/client", binding: "ASSETS" },
    vars: { LABEL: "default" },
    env: { staging: {
      name: "astro-staging", vars: { LABEL: "staging" },
      d1_databases: [{ binding: "DB", database_id: "00000000-0000-0000-0000-000000000000",
        database_name: "app", migrations_dir: "../db" }],
    } },
  };
  fs.writeFileSync(path.join(root, "config/wrangler.json"), JSON.stringify(wrangler));
  fs.writeFileSync(path.join(root, "db/001.sql"), "CREATE TABLE entries (value TEXT);");
  fs.writeFileSync(path.join(root, "astro.config.mjs"), `import { defineConfig } from "astro/config";
    import { tokamak } from ${JSON.stringify(new URL("./dist/astro.mjs", import.meta.url).href)};
    export default defineConfig({
      output: "server",
      adapter: tokamak({ name: "Astro Native", version: "1.0.0" }, {
        configPath: "config/wrangler.json", inspectorPort: false, persistState: false, remoteBindings: false,
      }),
      vite: { server: { watch: { usePolling: true } } },
    });`);
  fs.writeFileSync(path.join(root, "src/pages/index.astro"), `---
    import { env } from "cloudflare:workers";
    const message = await env.TOKAMAK.describe();
    ---
    <h1>{message}</h1>`);
  fs.writeFileSync(path.join(root, "src/shared.ts"), 'export const greeting = "first";');
  fs.writeFileSync(path.join(root, "src/tokamak.ts"), `import { WorkerEntrypoint } from "cloudflare:workers";
    import { greeting } from "./shared";
    export default class Native extends WorkerEntrypoint {
      describe(): string { return this.env.LABEL + " " + greeting; }
    }`);
  return { root, wrangler };
}

async function environment(root, run) {
  const values = {
    TOKAMAK_VITE_OUTPUT: path.join(root, ".tokamak/vite"),
    CLOUDFLARE_ENV: "staging",
    MINIFLARE_REGISTRY_PATH: path.join(root, "registry"),
  };
  const previous = Object.fromEntries(Object.keys(values).map(key => [key, process.env[key]]));
  Object.assign(process.env, values);
  try { return await run(); }
  finally {
    for (const [key, value] of Object.entries(previous)) {
      if (value === undefined) delete process.env[key];
      else process.env[key] = value;
    }
  }
}

test("Astro builds the worker graph while preserving environment settings and relative paths", async t => {
  const { root, wrangler } = project(t);
  delete wrangler.main;
  fs.writeFileSync(path.join(root, "config/wrangler.json"), JSON.stringify(wrangler));
  await environment(root, () => build({ root: pathToFileURL(`${root}/`), logLevel: "silent" }));
  const pointer = path.join(root, ".wrangler/deploy/config.json");
  const deploy = JSON.parse(fs.readFileSync(pointer, "utf8"));
  const main = JSON.parse(fs.readFileSync(path.resolve(path.dirname(pointer), deploy.configPath), "utf8"));
  const nativePath = path.resolve(path.dirname(pointer), deploy.auxiliaryWorkers[0].configPath);
  const native = JSON.parse(fs.readFileSync(nativePath, "utf8"));
  assert.deepEqual(main.services, [{ binding: "TOKAMAK", service: "astro-staging-tokamak" }]);
  assert.deepEqual(native.services, [{ binding: "MAIN", service: "astro-staging" }]);
  assert.equal(native.vars.LABEL, "staging");
  assert.ok(native.kv_namespaces.some(binding => binding.binding === "SESSION"));
  assert.equal(fs.readFileSync(path.resolve(path.dirname(nativePath), native.d1_databases[0].migrations_dir, "001.sql"), "utf8"),
    "CREATE TABLE entries (value TEXT);");
  assert.deepEqual(JSON.parse(fs.readFileSync(path.join(root, "config/wrangler.json"), "utf8")), wrangler);
  assert.deepEqual(fs.readdirSync(path.join(root, "config")), ["wrangler.json"]);
});

test("Astro development reloads native code and local bindings without exposing its configuration", async t => {
  const { root } = project(t);
  const vars = path.join(root, "config/.dev.vars.staging");
  fs.writeFileSync(vars, 'LABEL="local"');
  await environment(root, async () => {
    const server = await dev({ root: pathToFileURL(`${root}/`), logLevel: "silent", server: { port: 0 } });
    try {
      const report = JSON.parse(fs.readFileSync(path.join(root, ".tokamak/vite/server.json"), "utf8"));
      assert.equal(report.workerName, "astro-app");
      assert.match(await (await fetch(report.url)).text(), /<h1\b[^>]*>local first<\/h1>/);
      fs.writeFileSync(path.join(root, "src/shared.ts"), 'export const greeting = "second";');
      await waitForPage(root, "local second");
      fs.writeFileSync(vars, 'LABEL="reloaded"');
      await waitForPage(root, "reloaded second");
      assert.ok(fs.readdirSync(path.join(root, "config")).some(file => file.startsWith(".tokamak.")));
      const latest = JSON.parse(fs.readFileSync(path.join(root, ".tokamak/vite/server.json"), "utf8"));
      const response = await fetch(new URL(`/@fs/${path.join(root, "config/wrangler.json")}`, latest.url));
      assert.equal(response.status, 403);
    } finally {
      await server.stop();
    }
    assert.deepEqual(fs.readdirSync(path.join(root, "config")), [".dev.vars.staging", "wrangler.json"]);
  });
});

async function waitForPage(root, message) {
  let result;
  for (const deadline = Date.now() + 10_000; Date.now() < deadline;) {
    try {
      const report = JSON.parse(fs.readFileSync(path.join(root, ".tokamak/vite/server.json"), "utf8"));
      result = await (await fetch(report.url)).text();
      if (result.includes(message)) return;
    } catch (error) {
      result = String(error);
    }
    await new Promise(resolve => setTimeout(resolve, 50));
  }
  assert.fail(`Expected page to contain ${JSON.stringify(message)}; received ${result}`);
}
