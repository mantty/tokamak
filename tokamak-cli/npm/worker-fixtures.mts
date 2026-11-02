import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import type { TestContext } from "node:test";

import type { ViteDevServer } from "vite";
import { experimental_generateTypes } from "wrangler";

async function runtimeTypes() {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "tokamak-worker-types-"));
  try {
    const config = path.join(root, "wrangler.json");
    fs.writeFileSync(config, JSON.stringify({ name: "types", compatibility_date: "2026-09-01" }));
    const { runtime } = await experimental_generateTypes({
      config, includeEnv: false, path: path.join(root, "worker-configuration.d.ts"),
    });
    if (runtime === null) throw new Error("Wrangler generated no runtime types");
    return runtime;
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
}

export const workerTypes = await runtimeTypes();

/** A new temporary directory, removed after `t`, holding `files` by their paths in it. */
export function temporaryProject(t: TestContext, prefix: string, files: Record<string, string>): string {
  const root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), prefix)));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  writeFiles(root, files);
  return root;
}

/** The local URL that the development server `server` listens on. */
export function localURL(server: ViteDevServer): string {
  const url = server.resolvedUrls?.local[0];
  assert.ok(url, "the server has no local URL");
  return url;
}

/** Writes each of `files` at its path under `root`. */
export function writeFiles(root: string, files: Record<string, string>): void {
  for (const [name, contents] of Object.entries(files)) {
    fs.mkdirSync(path.dirname(path.join(root, name)), { recursive: true });
    fs.writeFileSync(path.join(root, name), contents);
  }
}
