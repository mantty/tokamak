"use strict";

const assert = require("node:assert/strict");
const { test } = require("node:test");

const { binaryPath, packageFor } = require("./platform.js");

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
