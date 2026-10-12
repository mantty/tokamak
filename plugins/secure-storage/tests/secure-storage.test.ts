import assert from "node:assert/strict";
import { afterEach, test } from "node:test";

import { connectNative, disconnectNative } from "@tokamakdev/plugin/testing";

import { secureStorage } from "../src/index.js";

afterEach(disconnectNative);

void test("rejects without a native bridge", async () => {
  await assert.rejects(
    secureStorage.get("identity"),
    (error: unknown) => error instanceof DOMException && error.name === "NotSupportedError",
  );
});

void test("stores a value with its options", async () => {
  const native = connectNative();

  const stored = secureStorage.set("identity", "c2VjcmV0", {
    readable: "whenUnlocked",
    authentication: "biometricsOrPasscode",
    prompt: "Confirm it's you",
    keychainGroup: "TEAMID.com.example.shared",
  });
  assert.deepEqual(native.lastRequest(), {
    type: "call",
    plugin: "secure-storage",
    method: "set",
    arguments: {
      name: "identity",
      value: "c2VjcmV0",
      readable: "whenUnlocked",
      authentication: "biometricsOrPasscode",
      thisDeviceOnly: true,
      keychainGroup: "TEAMID.com.example.shared",
      prompt: "Confirm it's you",
    },
  });
  native.respond({ value: null });

  await stored;
});

void test("stores a restorable value without authentication", async () => {
  const native = connectNative();

  const stored = secureStorage.set("identity", "c2VjcmV0", {
    readable: "afterFirstUnlock",
    thisDeviceOnly: false,
  });
  assert.deepEqual(native.lastRequest().arguments, {
    name: "identity",
    value: "c2VjcmV0",
    readable: "afterFirstUnlock",
    authentication: null,
    thisDeviceOnly: false,
    keychainGroup: null,
    prompt: null,
  });
  native.respond({ value: null });

  await stored;
});

void test("creates a value", async () => {
  const native = connectNative();

  const created = secureStorage.create("identity", "c2VjcmV0", {
    readable: "whenUnlocked",
    authentication: "biometrics",
    keychainGroup: "TEAMID.com.example.shared",
  });
  assert.deepEqual(native.lastRequest(), {
    type: "call",
    plugin: "secure-storage",
    method: "create",
    arguments: {
      name: "identity",
      value: "c2VjcmV0",
      readable: "whenUnlocked",
      authentication: "biometrics",
      thisDeviceOnly: true,
      keychainGroup: "TEAMID.com.example.shared",
    },
  });
  native.respond({ value: null });

  await created;
});

void test("replaces a value", async () => {
  const native = connectNative();

  const replaced = secureStorage.replace("identity", "c2VjcmV0", {
    readable: "whenUnlocked",
    prompt: "Confirm it's you",
  });
  assert.deepEqual(native.lastRequest(), {
    type: "call",
    plugin: "secure-storage",
    method: "replace",
    arguments: {
      name: "identity",
      value: "c2VjcmV0",
      readable: "whenUnlocked",
      authentication: null,
      thisDeviceOnly: true,
      keychainGroup: null,
      prompt: "Confirm it's you",
    },
  });
  native.respond({ value: null });

  await replaced;
});

void test("replaces a value without a prompt", async () => {
  const native = connectNative();

  const replaced = secureStorage.replace("identity", "c2VjcmV0", { readable: "whenUnlocked" });
  assert.deepEqual(native.lastRequest().arguments, {
    name: "identity",
    value: "c2VjcmV0",
    readable: "whenUnlocked",
    authentication: null,
    thisDeviceOnly: true,
    keychainGroup: null,
    prompt: null,
  });
  native.respond({ value: null });

  await replaced;
});

void test("reads a value with a prompt", async () => {
  const native = connectNative();

  const value = secureStorage.get("identity", {
    prompt: "Confirm it's you",
    keychainGroup: "TEAMID.com.example.shared",
  });
  assert.deepEqual(native.lastRequest(), {
    type: "call",
    plugin: "secure-storage",
    method: "get",
    arguments: { name: "identity", prompt: "Confirm it's you", keychainGroup: "TEAMID.com.example.shared" },
  });
  native.respond({ value: "c2VjcmV0" });

  assert.equal(await value, "c2VjcmV0");
});

void test("reads a missing value as null", async () => {
  const native = connectNative();

  const value = secureStorage.get("identity");
  assert.deepEqual(native.lastRequest().arguments, { name: "identity", prompt: null, keychainGroup: null });
  native.respond({ value: null });

  assert.equal(await value, null);
});

void test("deletes a value", async () => {
  const native = connectNative();

  const deleted = secureStorage.delete("identity");
  assert.deepEqual(native.lastRequest().arguments, { name: "identity", keychainGroup: null });
  native.respond({ value: null });

  await deleted;
});

void test("deletes a value in one keychain group", async () => {
  const native = connectNative();

  const deleted = secureStorage.delete("identity", { keychainGroup: "TEAMID.com.example.shared" });
  assert.deepEqual(native.lastRequest(), {
    type: "call",
    plugin: "secure-storage",
    method: "delete",
    arguments: { name: "identity", keychainGroup: "TEAMID.com.example.shared" },
  });
  native.respond({ value: null });

  await deleted;
});

void test("clears every value", async () => {
  const native = connectNative();

  const cleared = secureStorage.clear();
  assert.deepEqual(native.lastRequest(), {
    type: "call",
    plugin: "secure-storage",
    method: "clear",
    arguments: { keychainGroup: null },
  });
  native.respond({ value: null });

  await cleared;
});

void test("clears one keychain group", async () => {
  const native = connectNative();

  const cleared = secureStorage.clear({ keychainGroup: "TEAMID.*" });
  assert.deepEqual(native.lastRequest().arguments, { keychainGroup: "TEAMID.*" });
  native.respond({ value: null });

  await cleared;
});

void test("preserves native errors", async () => {
  const native = connectNative();

  const created = secureStorage.create("identity", "c2VjcmV0", { readable: "whenUnlocked" });
  native.respond({ error: { name: "DuplicateItemError", message: "The specified item already exists in the keychain." } });

  await assert.rejects(
    created,
    (error: unknown) =>
      error instanceof DOMException &&
      error.name === "DuplicateItemError" &&
      error.message === "The specified item already exists in the keychain.",
  );
});
