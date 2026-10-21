import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { test } from "node:test";
import { fileURLToPath, pathToFileURL } from "node:url";

import { build, dev } from "astro";
import { project } from "./astro-fixture.mjs";

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
  const { root, wrangler } = project(t, { sessionKVBindingName: "APP_SESSION" });
  const sdk = JSON.stringify(fileURLToPath(import.meta.resolve("@tokamakdev/tok")));
  fs.writeFileSync(path.join(root, "src/tokamak.ts"), `import { TokamakEntrypoint } from ${sdk};
    export default class Native extends TokamakEntrypoint {
      describe() { return this.env.APP_SESSION.get("description"); }
    }`);
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
  assert.ok(native.kv_namespaces.some(binding => binding.binding === "APP_SESSION"));
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
    const server = await dev({ root: pathToFileURL(`${root}/`), logLevel: "silent", server: { port: 0, host: "127.0.0.1" } });
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

test("Astro web development serves generated clients without optimizing the server entrypoint", async t => {
  const { root } = project(t);
  fs.writeFileSync(path.join(root, "src/pages/index.astro"),
    '<h1>Web client</h1><script>import client from "virtual:tokamak/client"; console.log(client);</script>');
  await environment(root, async () => {
    delete process.env.TOKAMAK_VITE_OUTPUT;
    const server = await dev({ root: pathToFileURL(`${root}/`), logLevel: "silent", server: { port: 0, host: "127.0.0.1" } });
    try {
      const origin = `http://127.0.0.1:${server.address.port}`;
      assert.equal((await fetch(origin)).status, 200);
      const response = await fetch(`${origin}/@id/__x00__virtual:tokamak/client`);
      const body = await response.text();
      assert.equal(response.status, 200, body);
      assert.match(body, /describe/);
    } finally {
      await server.stop();
    }
  });
});
