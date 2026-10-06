import assert from "node:assert/strict";
import { afterEach, test } from "node:test";

import { connectNative, disconnectNative } from "@tokamakdev/plugin/testing";

import { localAuthentication } from "../src/index.js";

afterEach(disconnectNative);

void test("rejects without a native bridge", async () => {
  await assert.rejects(
    localAuthentication.status("biometrics"),
    (error: unknown) => error instanceof DOMException && error.name === "NotSupportedError",
  );
});

void test("reports whether an authentication is available", async () => {
  const native = connectNative();

  const status = localAuthentication.status("biometricsOrPasscode");
  assert.deepEqual(native.lastRequest(), {
    type: "call",
    plugin: "local-authentication",
    method: "status",
    arguments: { authentication: "biometricsOrPasscode" },
  });
  native.respond({ value: "notEnrolled" });

  assert.equal(await status, "notEnrolled");
});

void test("authenticates the device owner with a prompt", async () => {
  const native = connectNative();

  const authenticated = localAuthentication.authenticate("biometrics", {
    prompt: "Confirm it's you",
  });
  assert.deepEqual(native.lastRequest(), {
    type: "call",
    plugin: "local-authentication",
    method: "authenticate",
    arguments: { authentication: "biometrics", prompt: "Confirm it's you" },
  });
  native.respond({ value: null });

  await authenticated;
});

void test("rejects a cancelled authentication", async () => {
  const native = connectNative();

  const authenticated = localAuthentication.authenticate("biometricsOrPasscode", {
    prompt: "Confirm it's you",
  });
  native.respond({ error: { name: "NotAllowedError", message: "Authentication was cancelled" } });

  await assert.rejects(
    authenticated,
    (error: unknown) => error instanceof DOMException && error.name === "NotAllowedError",
  );
});
