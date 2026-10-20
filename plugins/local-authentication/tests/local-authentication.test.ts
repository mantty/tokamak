import assert from "node:assert/strict";
import { test } from "node:test";

import { connectNative } from "@tokamakdev/plugin/testing";

import { localAuthentication as createApi } from "../src/index.js";


void test("requires a Tokamak invocation context", () => {
  // @ts-expect-error -- native APIs require a receiving invocation.
  assert.throws(() => createApi(), TypeError);
});

void test("reports whether an authentication is available", async () => {
  const native = connectNative();
  const localAuthentication = createApi(native.ctx);

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
  const localAuthentication = createApi(native.ctx);

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
  const localAuthentication = createApi(native.ctx);

  const authenticated = localAuthentication.authenticate("biometricsOrPasscode", {
    prompt: "Confirm it's you",
  });
  native.respond({ error: { name: "NotAllowedError", message: "Authentication was cancelled" } });

  await assert.rejects(
    authenticated,
    (error: unknown) => error instanceof DOMException && error.name === "NotAllowedError",
  );
});
