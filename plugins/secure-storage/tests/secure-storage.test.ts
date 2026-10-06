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
  });
  native.respond({ value: null });

  await stored;
});

void test("reads a value with a prompt", async () => {
  const native = connectNative();

  const value = secureStorage.get("identity", { prompt: "Confirm it's you" });
  assert.deepEqual(native.lastRequest(), {
    type: "call",
    plugin: "secure-storage",
    method: "get",
    arguments: { name: "identity", prompt: "Confirm it's you" },
  });
  native.respond({ value: "c2VjcmV0" });

  assert.equal(await value, "c2VjcmV0");
});

void test("reads a missing value as null", async () => {
  const native = connectNative();

  const value = secureStorage.get("identity");
  assert.deepEqual(native.lastRequest().arguments, { name: "identity", prompt: null });
  native.respond({ value: null });

  assert.equal(await value, null);
});

void test("deletes a value", async () => {
  const native = connectNative();

  const deleted = secureStorage.delete("identity");
  assert.deepEqual(native.lastRequest(), {
    type: "call",
    plugin: "secure-storage",
    method: "delete",
    arguments: { name: "identity" },
  });
  native.respond({ value: null });

  await deleted;
});

void test("lists the stored names", async () => {
  const native = connectNative();

  const names = secureStorage.keys();
  assert.deepEqual(native.lastRequest(), { type: "call", plugin: "secure-storage", method: "keys", arguments: null });
  native.respond({ value: ["device", "identity"] });

  assert.deepEqual(await names, ["device", "identity"]);
});

void test("clears every value", async () => {
  const native = connectNative();

  const cleared = secureStorage.clear();
  assert.deepEqual(native.lastRequest(), { type: "call", plugin: "secure-storage", method: "clear", arguments: null });
  native.respond({ value: null });

  await cleared;
});

void test("preserves native errors", async () => {
  const native = connectNative();

  const value = secureStorage.get("identity", { prompt: "Confirm it's you" });
  native.respond({ error: { name: "NotAllowedError", message: "Authentication was cancelled" } });

  await assert.rejects(
    value,
    (error: unknown) =>
      error instanceof DOMException &&
      error.name === "NotAllowedError" &&
      error.message === "Authentication was cancelled",
  );
});
