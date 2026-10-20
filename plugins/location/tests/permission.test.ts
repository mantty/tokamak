import assert from "node:assert/strict";
import { afterEach, test } from "node:test";

import { connectNative } from "@tokamakdev/plugin/testing";

import { location } from "../web/client.js";
import { location as createApi } from "../src/index.js";

const originalNavigator = Object.getOwnPropertyDescriptor(globalThis, "navigator");

afterEach(() => {
  if (originalNavigator) Object.defineProperty(globalThis, "navigator", originalNavigator);
  else Reflect.deleteProperty(globalThis, "navigator");
});

const granted = { inUse: "granted", background: "prompt", precision: "precise" };

void test("calls the native plugin with each permission method's arguments", async () => {
  const native = connectNative();
  const location = createApi(native.ctx);
  const cases: [() => Promise<unknown>, string, unknown][] = [
    [() => location.permission(), "permission", null],
    [() => location.requestPermission(), "requestPermission", {}],
    [
      () => location.requestPermission({ background: true, precision: "precise", purpose: "RunTracking" }),
      "requestPermission",
      { background: true, precision: "precise", purpose: "RunTracking" },
    ],
    [() => location.servicesEnabled(), "servicesEnabled", null],
    [() => location.openSettings(), "openSettings", null],
  ];
  for (const [run, method, arguments_] of cases) {
    const result = run();
    assert.deepEqual(native.lastRequest(), { type: "call", plugin: "location", method, arguments: arguments_ });
    native.respond({ value: granted });
    assert.deepEqual(await result, granted);
  }
});

void test("reports the web page's permission from the Permissions API", async () => {
  Object.defineProperty(globalThis, "navigator", {
    configurable: true,
    value: {
      permissions: {
        query: (descriptor: PermissionDescriptor) =>
          Promise.resolve({ state: descriptor.name === "geolocation" ? "granted" : "denied" }),
      },
    },
  });

  assert.deepEqual(await location.permission(), { inUse: "granted", background: "denied", precision: "precise" });
});
