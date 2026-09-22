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

function fakeInstall(script, mode, platformPacks = []) {
  const root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), "tok-launcher-")));
  const modules = path.join(root, "node_modules");
  const bin = path.join(modules, packageFor(process.platform, process.arch), "bin");
  fs.mkdirSync(bin, { recursive: true });
  fs.writeFileSync(path.join(bin, "tok"), script, { mode });
  for (const name of platformPacks) {
    fs.mkdirSync(path.join(modules, name), { recursive: true });
    fs.writeFileSync(path.join(modules, name, "package.json"), "{}");
  }
  const env = { ...process.env, NODE_PATH: modules };
  delete env.TOKAMAK_PLATFORM_PACK_PATH;
  return { env, modules };
}

const PRINT_PLATFORM_PACK_PATH = "#!/bin/sh\nprintf '%s' \"$TOKAMAK_PLATFORM_PACK_PATH\"\n";

test("forwards arguments and the binary's exit status", unix, () => {
  const { env } = fakeInstall("#!/bin/sh\necho \"$@\"\nexit 7\n", 0o755);
  const result = spawnSync(process.execPath, [LAUNCHER, "build", "macos"], { env, encoding: "utf8" });
  assert.equal(result.status, 7);
  assert.equal(result.stdout, "build macos\n");
});

test("reports a binary that cannot be started", unix, () => {
  const { env } = fakeInstall("not executable", 0o644);
  const result = spawnSync(process.execPath, [LAUNCHER], { env, encoding: "utf8" });
  assert.equal(result.status, 1);
  assert.match(result.stderr, /could not run .*bin\/tok: /);
});

test("leaves the platform-pack path unset without installed platform packs", unix, () => {
  const { env } = fakeInstall(PRINT_PLATFORM_PACK_PATH, 0o755);
  const result = spawnSync(process.execPath, [LAUNCHER], { env, encoding: "utf8" });
  assert.equal(result.stdout, "");
});

test("passes the installed platform packs to the binary", unix, () => {
  const packs = ["@tokamakdev/platform-android-arm64", "@tokamakdev/platform-macos-arm64"];
  const { env, modules } = fakeInstall(PRINT_PLATFORM_PACK_PATH, 0o755, packs);
  const result = spawnSync(process.execPath, [LAUNCHER], { env, encoding: "utf8" });
  assert.equal(result.status, 0);
  assert.equal(result.stdout, packs.map((name) => path.join(modules, name)).join(path.delimiter));
});

test("keeps a platform-pack path set by the user", unix, () => {
  const { env } = fakeInstall(PRINT_PLATFORM_PACK_PATH, 0o755, ["@tokamakdev/platform-macos-arm64"]);
  env.TOKAMAK_PLATFORM_PACK_PATH = "/custom/platform-packs";
  const result = spawnSync(process.execPath, [LAUNCHER], { env, encoding: "utf8" });
  assert.equal(result.stdout, "/custom/platform-packs");
});
