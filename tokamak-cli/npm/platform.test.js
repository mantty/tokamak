"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const { test } = require("node:test");

const { PLATFORM_PACKS, binaryPath, packageFor, platformPackRoots } = require("./platform.js");

test("maps supported platforms to their binary packages", () => {
  assert.equal(packageFor("darwin", "arm64"), "@tokamakdev/tok-darwin-arm64");
  assert.equal(packageFor("darwin", "x64"), "@tokamakdev/tok-darwin-x64");
  assert.equal(packageFor("linux", "x64"), "@tokamakdev/tok-linux-x64");
  assert.equal(packageFor("win32", "x64"), "@tokamakdev/tok-win32-x64");
});

test("rejects platforms without a published CLI", () => {
  assert.throws(() => packageFor("linux", "arm64"), /does not publish a CLI for linux arm64/);
});

test("resolves the binary inside the platform package", () => {
  const resolved = [];
  const resolve = (request) => {
    resolved.push(request);
    return `/modules/${request}`;
  };
  assert.equal(binaryPath("win32", "x64", resolve), "/modules/@tokamakdev/tok-win32-x64/bin/tok.exe");
  assert.equal(binaryPath("linux", "x64", resolve), "/modules/@tokamakdev/tok-linux-x64/bin/tok");
  assert.deepEqual(resolved, ["@tokamakdev/tok-win32-x64/bin/tok.exe", "@tokamakdev/tok-linux-x64/bin/tok"]);
});

test("explains a missing platform package", () => {
  const resolve = () => {
    throw new Error("Cannot find module");
  };
  assert.throws(() => binaryPath("darwin", "arm64", resolve), /@tokamakdev\/tok-darwin-arm64 is not installed/);
});

test("lists every platform-pack package as an optional dependency", () => {
  const targets = fs.readdirSync(path.join(__dirname, "platform-packs")).sort();
  assert.deepEqual(PLATFORM_PACKS, targets.map((target) => `@tokamakdev/platform-${target}`));
});

test("returns the directories of installed platform packs", () => {
  const installed = new Set(["@tokamakdev/platform-android-arm64", "@tokamakdev/platform-windows-x64"]);
  const resolve = (request) => {
    const name = request.slice(0, -"/package.json".length);
    if (!installed.has(name)) throw new Error("Cannot find module");
    return `/modules/${request}`;
  };
  assert.deepEqual(platformPackRoots(resolve), [
    "/modules/@tokamakdev/platform-android-arm64",
    "/modules/@tokamakdev/platform-windows-x64",
  ]);
});
