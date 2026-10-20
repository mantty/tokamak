import assert from "node:assert/strict";
import { test } from "node:test";

import { connectNative } from "@tokamakdev/plugin/testing";

import { location as createApi } from "../src/index.js";


const home = { id: "home", latitude: 51.5, longitude: -0.1, radius: 100 };

void test("calls the native plugin with each region method's arguments", async () => {
  const native = connectNative();
  const location = createApi(native.ctx);
  const cases: [() => Promise<unknown>, string, unknown][] = [
    [() => location.addRegion(home), "addRegion", home],
    [() => location.addRegion({ ...home, onExit: false }), "addRegion", { ...home, onExit: false }],
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

