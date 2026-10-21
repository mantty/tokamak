import { decodeValues } from "@tokamakdev/plugin/streams";
import assert from "node:assert/strict";
import { test } from "node:test";

import { connectNative, dispatch } from "@tokamakdev/plugin/testing";

import { location as createApi, type LocationUpdate } from "../src/index.js";
import { onLocationError, onLocationUpdate, onRegionTransition, onVisit } from "../events/index.js";


void test("calls the native plugin with each update and setup method's arguments", async () => {
  const native = connectNative();
  const location = createApi(native.ctx);
  const options = {
    activity: "fitness",
    accuracy: "balanced",
    distance: 5.5,
    interval: 1000,
    foregroundService: { title: "Recording your run", body: "", channel: "Run recording" },
  } as const;
  const cases: [() => Promise<unknown>, string, unknown][] = [
    [() => location.startUpdates(), "startUpdates", {}],
    [() => location.startUpdates(options), "startUpdates", options],
    [() => location.stopUpdates(), "stopUpdates", null],
    [() => location.getUpdates(), "getUpdates", null],
    [() => location.setup({}), "setup", {}],
    [
      () => location.setup({ significantChanges: true, visits: false }),
      "setup",
      { significantChanges: true, visits: false },
    ],
  ];
  for (const [run, method, arguments_] of cases) {
    const result = run();
    assert.deepEqual(native.lastRequest(), { type: "call", plugin: "location", method, arguments: arguments_ });
    native.respond({ value: null });
    assert.equal(await result, null);
  }
});

void test("each native event stream delivers values until cancelled", async () => {
  const native = connectNative();
  const location = createApi(native.ctx);
  const update: LocationUpdate = { positions: [], stationary: false };
  for (const method of ["onLocationUpdate", "onRegionTransition", "onVisit", "onLocationError"] as const) {
    const values = decodeValues<unknown>(location[method]());
    assert.deepEqual(native.lastRequest(), { type: "subscribe", plugin: "location", method, arguments: null });
    native.respond({ value: update }, false);
    assert.deepEqual((await values.next()).value, update);
    await values.return(undefined);
    assert.equal(native.sent.at(-1)?.type, "cancel");
  }
});

void test("native event stream failures reject their readers", async () => {
  const native = connectNative();
  const location = createApi(native.ctx);
  for (const method of ["onLocationUpdate", "onRegionTransition", "onVisit", "onLocationError"] as const) {
    const values = decodeValues<unknown>(location[method]());
    native.respond({ error: { name: "NotSupportedError", message: "Not here" } });
    await assert.rejects(values.next(), { name: "NotSupportedError" });
  }
});




void test("delivers each Worker event to its listeners", async () => {
  const received: string[] = [];
  onLocationUpdate((update) => {
    received.push(`update ${String(update.positions.length)}`);
  });
  onRegionTransition((transition) => {
    received.push(`${transition.transition} ${transition.id}`);
  });
  onVisit((visit) => {
    received.push(`visit ${String(visit.arrival)}`);
  });
  onLocationError((error) => {
    received.push(error.name);
  });

  await dispatch("location.update", { positions: [] });
  await dispatch("location.region", { id: "home", transition: "enter", timestamp: 1 });
  await dispatch("location.visit", { latitude: 1, longitude: 2, accuracy: 3, arrival: 4, departure: null });
  const { listened } = await dispatch("location.error", { name: "NotReadableError", message: "Off" });

  assert.deepEqual(received, ["update 0", "enter home", "visit 4", "NotReadableError"]);
  assert.deepEqual(
    ["location.update", "location.region", "location.visit", "location.error"].filter((name) => listened.includes(name)),
    ["location.update", "location.region", "location.visit", "location.error"],
  );
});
