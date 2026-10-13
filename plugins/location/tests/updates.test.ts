import assert from "node:assert/strict";
import { afterEach, test } from "node:test";

import { connectNative, disconnectNative, dispatch } from "@tokamakdev/plugin/testing";

import { location, type LocationUpdate, type SetupConfig, type UpdateOptions } from "../src/index.js";
import { onLocationError, onLocationUpdate, onRegionTransition, onVisit } from "../events/index.js";

afterEach(disconnectNative);

void test("calls the native plugin with each update and setup method's arguments", async () => {
  const native = connectNative();
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

void test("rejects update options it cannot send", async () => {
  connectNative();
  for (const [options, message] of [
    [{ activity: "walking" }, "options.activity must be one of"],
    [{ accuracy: "best" }, 'options.accuracy must be one of "high", "balanced", "low"'],
    [{ distance: -1 }, "options.distance must be a finite number of at least 0"],
    [{ distance: Infinity }, "options.distance must be a finite number of at least 0"],
    [{ interval: 1.5 }, "options.interval must be a whole number of at least 0"],
    [{ foregroundService: { title: "Run", body: "" } }, "options.foregroundService.channel must be a non-empty string"],
    [{ foregroundService: { title: "", body: "", channel: "Runs" } }, "options.foregroundService.title must be"],
    [{ foregroundService: { title: "Run", body: 1, channel: "Runs" } }, "options.foregroundService.body must be a string"],
  ] as const) {
    await assert.rejects(
      location.startUpdates(options as unknown as UpdateOptions),
      (error: unknown) => error instanceof TypeError && error.message.startsWith(message),
    );
  }
});

void test("rejects a setup configuration it cannot apply", async () => {
  connectNative();
  for (const [config, message] of [
    [{ visits: "yes" }, "config.visits must be a boolean"],
    [null, "config must be an object"],
  ] as const) {
    await assert.rejects(
      location.setup(config as unknown as SetupConfig),
      (error: unknown) => error instanceof TypeError && error.message === message,
    );
  }
});

void test("subscribes each listener to its native events until removed", () => {
  const native = connectNative();
  const update: LocationUpdate = {
    positions: [{ coords: { latitude: 1, longitude: 2, accuracy: 3, altitude: null, altitudeAccuracy: null, heading: null, speed: null }, timestamp: 4 }],
    stationary: false,
  };
  const listeners = [
    ["onLocationUpdate", (listener: (value: unknown) => void) => location.onLocationUpdate(listener)],
    ["onRegionTransition", (listener: (value: unknown) => void) => location.onRegionTransition(listener)],
    ["onVisit", (listener: (value: unknown) => void) => location.onVisit(listener)],
    ["onLocationError", (listener: (value: unknown) => void) => location.onLocationError(listener)],
  ] as const;
  for (const [method, listen] of listeners) {
    const received: unknown[] = [];
    const remove = listen((value) => received.push(value));
    const { session, id } = native.sent.at(-1) ?? {};
    assert.deepEqual(native.lastRequest(), { type: "subscribe", plugin: "location", method, arguments: null });

    native.respond({ value: update }, false);
    remove();

    assert.deepEqual(received, [update]);
    assert.deepEqual(native.sent.at(-1), { type: "cancel", session, id });
  }
});

void test("passes native listener failures to each listener's error callback", () => {
  const native = connectNative();
  const listeners = [
    (error: (failure: DOMException) => void) => location.onLocationUpdate(() => assert.fail(), error),
    (error: (failure: DOMException) => void) => location.onRegionTransition(() => assert.fail(), error),
    (error: (failure: DOMException) => void) => location.onVisit(() => assert.fail(), error),
    (error: (failure: DOMException) => void) => location.onLocationError(() => assert.fail(), error),
  ];
  for (const listen of listeners) {
    const received: string[] = [];
    listen((failure) => received.push(failure.name));

    native.respond({ error: { name: "NotSupportedError", message: "Not here" } });

    assert.deepEqual(received, ["NotSupportedError"]);
  }
});

void test("checks a setup configuration before the web rejects it", async () => {
  await assert.rejects(location.setup({ visits: "yes" } as unknown as SetupConfig), TypeError);
});

void test("checks update options before the web rejects them", async () => {
  for (const options of [null, { foregroundService: null }, { interval: -1 }]) {
    await assert.rejects(
      location.startUpdates(options as unknown as UpdateOptions),
      (error: unknown) => error instanceof TypeError,
    );
  }
});

void test("rejects updates on the web, whose listeners receive nothing", async () => {
  const unsupported = (error: unknown) => error instanceof DOMException && error.name === "NotSupportedError";

  await assert.rejects(location.startUpdates(), unsupported);
  await assert.rejects(location.stopUpdates(), unsupported);
  await assert.rejects(location.getUpdates(), unsupported);
  await assert.rejects(location.setup({}), unsupported);
  for (const remove of [
    location.onLocationUpdate(() => assert.fail()),
    location.onRegionTransition(() => assert.fail()),
    location.onVisit(() => assert.fail()),
    location.onLocationError(() => assert.fail()),
  ]) {
    remove();
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
