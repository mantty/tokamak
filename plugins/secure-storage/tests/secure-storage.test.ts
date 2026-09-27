import assert from "node:assert/strict";
import { afterEach, test } from "node:test";

import { secureStorage } from "../src/index.js";

type Message = Record<string, unknown>;

afterEach(() => {
  Reflect.deleteProperty(globalThis, "__tokamakNative");
  Reflect.deleteProperty(globalThis, "__tokamakReceive");
});

function connectNative(): Message[] {
  const sent: Message[] = [];
  globalThis.__tokamakNative = {
    onmessage: null,
    postMessage(message) {
      sent.push(JSON.parse(message) as Message);
    },
  };
  return sent;
}

function lastCall(sent: Message[]): Message {
  const { plugin, method, arguments: arguments_ } = sent.at(-1) ?? {};
  return { plugin, method, arguments: arguments_ };
}

function respond(sent: Message[], response: { value?: unknown; error?: { name: string; message: string } }) {
  const request = sent.at(-1);
  globalThis.__tokamakReceive?.({
    session: request?.session as string,
    id: request?.id as number,
    done: true,
    ...response,
  });
}

void test("rejects without a native bridge", async () => {
  await assert.rejects(
    secureStorage.get("identity"),
    (error: unknown) => error instanceof DOMException && error.name === "NotSupportedError",
  );
});

void test("stores a value with its options", async () => {
  const sent = connectNative();

  const stored = secureStorage.set("identity", "c2VjcmV0", {
    readable: "whenUnlocked",
    authentication: "biometricsOrPasscode",
  });
  assert.deepEqual(lastCall(sent), {
    plugin: "secure-storage",
    method: "set",
    arguments: {
      name: "identity",
      value: "c2VjcmV0",
      readable: "whenUnlocked",
      authentication: "biometricsOrPasscode",
    },
  });
  respond(sent, { value: null });

  await stored;
});

void test("stores a value without authentication", async () => {
  const sent = connectNative();

  const stored = secureStorage.set("identity", "c2VjcmV0", { readable: "afterFirstUnlock" });
  assert.deepEqual(lastCall(sent).arguments, {
    name: "identity",
    value: "c2VjcmV0",
    readable: "afterFirstUnlock",
    authentication: null,
  });
  respond(sent, { value: null });

  await stored;
});

void test("reads a value with a prompt", async () => {
  const sent = connectNative();

  const value = secureStorage.get("identity", { prompt: "Confirm it's you" });
  assert.deepEqual(lastCall(sent), {
    plugin: "secure-storage",
    method: "get",
    arguments: { name: "identity", prompt: "Confirm it's you" },
  });
  respond(sent, { value: "c2VjcmV0" });

  assert.equal(await value, "c2VjcmV0");
});

void test("reads a missing value as null", async () => {
  const sent = connectNative();

  const value = secureStorage.get("identity");
  assert.deepEqual(lastCall(sent).arguments, { name: "identity", prompt: null });
  respond(sent, { value: null });

  assert.equal(await value, null);
});

void test("deletes a value", async () => {
  const sent = connectNative();

  const deleted = secureStorage.delete("identity");
  assert.deepEqual(lastCall(sent), {
    plugin: "secure-storage",
    method: "delete",
    arguments: { name: "identity" },
  });
  respond(sent, { value: null });

  await deleted;
});

void test("preserves native errors", async () => {
  const sent = connectNative();

  const value = secureStorage.get("identity", { prompt: "Confirm it's you" });
  respond(sent, { error: { name: "NotAllowedError", message: "Authentication was cancelled" } });

  await assert.rejects(
    value,
    (error: unknown) =>
      error instanceof DOMException &&
      error.name === "NotAllowedError" &&
      error.message === "Authentication was cancelled",
  );
});
