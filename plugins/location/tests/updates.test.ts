import { decodeValues } from "@tokamakdev/plugin/streams";
import assert from "node:assert/strict";
import { test } from "node:test";

import { connectNative } from "@tokamakdev/plugin/testing";
import { TokamakEntrypoint } from "@tokamakdev/plugin/worker";

import {
  location as createApi,
  type LocationErrorEvent,
  type LocationUpdate,
  type RegionTransition,
  type Visit,
} from "../src/index.js";

void test("calls the native plugin with each update, significant change and visit method's arguments", async () => {
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
    [() => location.startUpdates(options), "startUpdates", { options }],
    [() => location.stopUpdates(), "stopUpdates", {}],
    [() => location.getUpdates(), "getUpdates", {}],
    [() => location.startSignificantChanges(), "startSignificantChanges", {}],
    [() => location.stopSignificantChanges(), "stopSignificantChanges", {}],
    [() => location.startVisits(), "startVisits", {}],
    [() => location.stopVisits(), "stopVisits", {}],
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
    assert.deepEqual(native.lastRequest(), { type: "subscribe", plugin: "location", method, arguments: {} });
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

void test("types the Worker's handlers with the events of both platform families", () => {
  class Native extends TokamakEntrypoint {
    _onLocationUpdate({ positions, stationary }: LocationUpdate) {
      return void [positions, stationary];
    }
    _onRegionTransition({ id, transition }: RegionTransition) {
      return void [id, transition];
    }
    _onVisit({ arrival, departure }: Visit) {
      return void [arrival, departure];
    }
    _onLocationError({ name, id }: LocationErrorEvent) {
      return void [name, id];
    }
  }
  class Mistyped extends TokamakEntrypoint {
    // @ts-expect-error -- the event is a `LocationUpdate`.
    _onLocationUpdate(update: string) {
      return void update;
    }
  }
  assert.ok(Native.prototype instanceof TokamakEntrypoint && Mistyped.prototype instanceof TokamakEntrypoint);
});
