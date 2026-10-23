import assert from "node:assert/strict";
import { test } from "node:test";

import { connectNative } from "@tokamakdev/plugin/testing";

import { secureStorage as createApi } from "../src/index.js";

void test("requires a Tokamak invocation context", () => {
  // @ts-expect-error -- native APIs require a receiving invocation.
  assert.throws(() => createApi(), TypeError);
});

void test("stores, creates and replaces a value with its options", async () => {
  const native = connectNative();
  const secureStorage = createApi(native.ctx);
  const options = {
    readable: "whenUnlocked",
    authentication: "biometricsOrPasscode",
    prompt: "Confirm it's you",
    keychainGroup: "TEAMID.com.example.shared",
  } as const;

  for (const method of ["set", "create", "replace"] as const) {
    const stored = secureStorage[method]("identity", "c2VjcmV0", options);
    assert.deepEqual(native.lastRequest(), {
      type: "call",
      plugin: "secure-storage",
      method,
      arguments: { name: "identity", value: "c2VjcmV0", options: { ...options, thisDeviceOnly: true } },
    });
    native.respond({ value: null });
    await stored;
  }

  const portable = secureStorage.create("identity", "c2VjcmV0", { readable: "afterFirstUnlock", thisDeviceOnly: false });
  assert.deepEqual(native.lastRequest().arguments, {
    name: "identity",
    value: "c2VjcmV0",
    options: { readable: "afterFirstUnlock", thisDeviceOnly: false },
  });
  native.respond({ value: null });
  await portable;
});

void test("reads a value with and without options", async () => {
  const native = connectNative("android");
  const secureStorage = createApi(native.ctx);

  const prompted = secureStorage.get("identity", { prompt: "Confirm it's you" });
  assert.deepEqual(native.lastRequest(), {
    type: "call",
    plugin: "secure-storage",
    method: "get",
    arguments: { name: "identity", options: { prompt: "Confirm it's you" } },
  });
  native.respond({ value: "c2VjcmV0" });
  assert.equal(await prompted, "c2VjcmV0");

  const value = secureStorage.get("identity");
  assert.deepEqual(native.lastRequest().arguments, { name: "identity" });
  native.respond({ value: "c2VjcmV0" });
  assert.equal(await value, "c2VjcmV0");
});

void test("deletes a value and clears a keychain group", async () => {
  const native = connectNative("macos");
  const secureStorage = createApi(native.ctx);

  const deleted = secureStorage.delete("identity");
  assert.deepEqual(native.lastRequest(), {
    type: "call",
    plugin: "secure-storage",
    method: "delete",
    arguments: { name: "identity" },
  });
  native.respond({ value: null });
  await deleted;

  const cleared = secureStorage.clear({ keychainGroup: "TEAMID.*" });
  assert.deepEqual(native.lastRequest().arguments, { options: { keychainGroup: "TEAMID.*" } });
  native.respond({ value: null });
  await cleared;
});

void test("preserves native errors", async () => {
  const native = connectNative();
  const secureStorage = createApi(native.ctx);

  const created = secureStorage.create("identity", "c2VjcmV0", { readable: "whenUnlocked" });
  native.respond({ error: { name: "DuplicateItemError", message: "The specified item already exists in the keychain." } });

  await assert.rejects(created, { name: "DuplicateItemError", message: "The specified item already exists in the keychain." });
});

void test("rejects on platforms without secure storage", async () => {
  const native = connectNative("windows");
  await assert.rejects(createApi(native.ctx).get("identity"), { name: "NotSupportedError" });
  assert.deepEqual(native.sent, []);
});
