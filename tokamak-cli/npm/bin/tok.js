#!/usr/bin/env node
"use strict";

const { spawnSync } = require("node:child_process");
const path = require("node:path");
const { binaryPath, platformPackRoots } = require("../platform.js");

let binary;
try {
  binary = binaryPath(process.platform, process.arch, require.resolve);
} catch (error) {
  console.error(error.message);
  process.exit(1);
}

// The terminal delivers Ctrl-C to both processes; tok shuts down gracefully on its own,
// so the launcher only waits for it.
for (const signal of ["SIGINT", "SIGTERM", "SIGHUP"]) process.on(signal, () => {});

const env = { ...process.env };
if (!env.TOKAMAK_PLATFORM_PACK_PATH) {
  const roots = platformPackRoots(require.resolve);
  if (roots.length > 0) env.TOKAMAK_PLATFORM_PACK_PATH = roots.join(path.delimiter);
}

const result = spawnSync(binary, process.argv.slice(2), { stdio: "inherit", env });
if (result.error) {
  console.error(`could not run ${binary}: ${result.error.message}`);
  process.exit(1);
}
if (result.signal) process.kill(process.pid, result.signal);
process.exit(result.status ?? 1);
