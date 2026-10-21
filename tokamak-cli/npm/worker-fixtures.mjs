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
