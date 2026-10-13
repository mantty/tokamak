import assert from "node:assert/strict";
import { afterEach, test } from "node:test";

import { connectNative, disconnectNative } from "@tokamakdev/plugin/testing";

import { location, type Region } from "../src/index.js";

afterEach(disconnectNative);

const home = { id: "home", latitude: 51.5, longitude: -0.1, radius: 100 };

void test("calls the native plugin with each region method's arguments", async () => {
  const native = connectNative();
  const cases: [() => Promise<unknown>, string, unknown][] = [
    [() => location.addRegion(home), "addRegion", { ...home, onEntry: true, onExit: true }],
    [
      () => location.addRegion({ ...home, onExit: false }),
      "addRegion",
      { ...home, onEntry: true, onExit: false },
    ],
    [() => location.getRegions(), "getRegions", null],
    [() => location.removeRegion("home"), "removeRegion", { id: "home" }],
  ];
  for (const [run, method, arguments_] of cases) {
    const result = run();
    assert.deepEqual(native.lastRequest(), { type: "call", plugin: "location", method, arguments: arguments_ });
    native.respond({ value: null });
    await result;
  }
});

void test("rejects regions it cannot monitor", async () => {
  connectNative();
  for (const [region, message] of [
    [{ ...home, id: "" }, "region.id must be a non-empty string"],
    [{ ...home, latitude: 91 }, "region.latitude must be a number from -90 to 90"],
    [{ ...home, longitude: -181 }, "region.longitude must be a number from -180 to 180"],
    [{ ...home, radius: 0 }, "region.radius must be a finite number greater than 0"],
    [{ ...home, onEntry: "yes" }, "region.onEntry must be a boolean"],
    [{ ...home, onEntry: false, onExit: false }, "region must report entry, exit or both"],
    [{ latitude: 1, longitude: 2, radius: 3 }, "region.id must be a non-empty string"],
  ] as const) {
    await assert.rejects(
      location.addRegion(region as unknown as Region),
      (error: unknown) => error instanceof TypeError && error.message === message,
    );
  }
  await assert.rejects(
    location.removeRegion(""),
    (error: unknown) => error instanceof TypeError && error.message === "id must be a non-empty string",
  );
});

void test("rejects regions on the web", async () => {
  const unsupported = (error: unknown) => error instanceof DOMException && error.name === "NotSupportedError";

  await assert.rejects(location.addRegion(home), unsupported);
  await assert.rejects(location.getRegions(), unsupported);
  await assert.rejects(location.removeRegion("home"), unsupported);
});
