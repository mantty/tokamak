import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { workerTypes } from "./worker-fixtures.mjs";

export function project(t, options = {}) {
  const root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), "tokamak-astro-")));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  for (const directory of ["src/pages", "config", "db"]) fs.mkdirSync(path.join(root, directory), { recursive: true });
  fs.symlinkSync(fileURLToPath(new URL("./node_modules", import.meta.url)), path.join(root, "node_modules"), "junction");
  fs.writeFileSync(path.join(root, "package.json"), JSON.stringify({ name: "astro-fixture", type: "module" }));
  fs.writeFileSync(path.join(root, "worker-configuration.d.ts"), `${workerTypes}
    declare namespace Cloudflare {
      interface Env { LABEL: string; DB: D1Database; ${JSON.stringify(options.sessionKVBindingName ?? "SESSION")}: KVNamespace }
    }`);
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
        ...${JSON.stringify(options)},
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
