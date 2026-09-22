"use strict";

const assert = require("node:assert/strict");
const { spawnSync } = require("node:child_process");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { test } = require("node:test");

const { packageFor } = require("./platform.js");

const LAUNCHER = path.join(__dirname, "bin", "tok.js");
const unix = { skip: process.platform === "win32" };

function fakePlatformPackage(script, mode) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "tok-launcher-"));
  const bin = path.join(root, "node_modules", packageFor(process.platform, process.arch), "bin");
  fs.mkdirSync(bin, { recursive: true });
  fs.writeFileSync(path.join(bin, "tok"), script, { mode });
  return { ...process.env, NODE_PATH: path.join(root, "node_modules") };
}

test("forwards arguments and the binary's exit status", unix, () => {
  const env = fakePlatformPackage("#!/bin/sh\necho \"$@\"\nexit 7\n", 0o755);
  const result = spawnSync(process.execPath, [LAUNCHER, "build", "macos"], { env, encoding: "utf8" });
  assert.equal(result.status, 7);
  assert.equal(result.stdout, "build macos\n");
});

test("reports a binary that cannot be started", unix, () => {
  const env = fakePlatformPackage("not executable", 0o644);
  const result = spawnSync(process.execPath, [LAUNCHER], { env, encoding: "utf8" });
  assert.equal(result.status, 1);
  assert.match(result.stderr, /could not run .*bin\/tok: /);
});
