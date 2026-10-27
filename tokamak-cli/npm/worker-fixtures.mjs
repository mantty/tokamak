import fs from "node:fs";
import os from "node:os";
import path from "node:path";

import { experimental_generateTypes } from "wrangler";

async function runtimeTypes() {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "tokamak-worker-types-"));
  try {
    const config = path.join(root, "wrangler.json");
    fs.writeFileSync(config, JSON.stringify({ name: "types", compatibility_date: "2026-09-01" }));
    const result = await experimental_generateTypes({
      config, includeEnv: false, path: path.join(root, "worker-configuration.d.ts"),
    });
    return result.runtime;
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
}

export const workerTypes = await runtimeTypes();

/** A new temporary directory, removed after `t`, holding `files` by their paths in it. */
export function temporaryProject(t, prefix, files) {
  const root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), prefix)));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  writeFiles(root, files);
  return root;
}

/** Writes each of `files` at its path under `root`. */
export function writeFiles(root, files) {
  for (const [name, contents] of Object.entries(files)) {
    fs.mkdirSync(path.dirname(path.join(root, name)), { recursive: true });
    fs.writeFileSync(path.join(root, name), contents);
  }
}
