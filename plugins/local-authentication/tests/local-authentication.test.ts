import assert from "node:assert/strict";
import { afterEach, test } from "node:test";

import { localAuthentication } from "../src/index.js";

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
    localAuthentication.status("biometrics"),
    (error: unknown) => error instanceof DOMException && error.name === "NotSupportedError",
  );
});

void test("reports whether an authentication is available", async () => {
  const sent = connectNative();

  const status = localAuthentication.status("biometricsOrPasscode");
  assert.deepEqual(lastCall(sent), {
    plugin: "local-authentication",
    method: "status",
    arguments: { authentication: "biometricsOrPasscode" },
  });
  respond(sent, { value: "notEnrolled" });

  assert.equal(await status, "notEnrolled");
});

void test("authenticates the device owner with a prompt", async () => {
  const sent = connectNative();

  const authenticated = localAuthentication.authenticate("biometrics", {
    prompt: "Confirm it's you",
  });
  assert.deepEqual(lastCall(sent), {
    plugin: "local-authentication",
    method: "authenticate",
    arguments: { authentication: "biometrics", prompt: "Confirm it's you" },
  });
  respond(sent, { value: null });

  await authenticated;
});

void test("rejects a cancelled authentication", async () => {
  const sent = connectNative();

  const authenticated = localAuthentication.authenticate("biometricsOrPasscode", {
    prompt: "Confirm it's you",
  });
  respond(sent, { error: { name: "NotAllowedError", message: "Authentication was cancelled" } });

  await assert.rejects(
    authenticated,
    (error: unknown) => error instanceof DOMException && error.name === "NotAllowedError",
  );
});
