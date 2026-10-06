import assert from "node:assert/strict";
import { afterEach, test } from "node:test";

import { connectNative, disconnectNative } from "@tokamakdev/plugin/testing";

import { location } from "../src/index.js";

const originalNavigator = Object.getOwnPropertyDescriptor(globalThis, "navigator");

afterEach(() => {
  disconnectNative();
  if (originalNavigator) {
    Object.defineProperty(globalThis, "navigator", originalNavigator);
  } else {
    Reflect.deleteProperty(globalThis, "navigator");
  }
});

void test("uses browser location on the web", async () => {
  Object.defineProperty(globalThis, "navigator", {
    configurable: true,
    value: {
      geolocation: {
        getCurrentPosition(success: PositionCallback) {
          success(browserPosition(51.5, -0.1));
        },
      },
    },
  });

  const position = await location.getCurrentPosition();

  assert.equal(position.coords.latitude, 51.5);
  assert.equal(position.coords.longitude, -0.1);
});

void test("reports unavailable web location as unsupported", async () => {
  Reflect.deleteProperty(globalThis, "navigator");

  await assert.rejects(
    location.getCurrentPosition(),
    (error: unknown) =>
      error instanceof DOMException && error.name === "NotSupportedError",
  );
});

for (const [code, name] of [
  [1, "NotAllowedError"],
  [2, "NotReadableError"],
  [3, "TimeoutError"],
] as const) {
  void test(`maps browser location error ${String(code)}`, async () => {
    Object.defineProperty(globalThis, "navigator", {
      configurable: true,
      value: {
        geolocation: {
          getCurrentPosition(_: PositionCallback, failure: (error: GeolocationPositionError) => void) {
            failure({
              code,
              message: "Location failed",
              PERMISSION_DENIED: 1,
              POSITION_UNAVAILABLE: 2,
              TIMEOUT: 3,
            });
          },
        },
      },
    });

    await assert.rejects(
      location.getCurrentPosition(),
      (error: unknown) => error instanceof DOMException && error.name === name,
    );
  });
}

void test("forwards browser watch failures", () => {
  Object.defineProperty(globalThis, "navigator", {
    configurable: true,
    value: {
      geolocation: {
        watchPosition(_: PositionCallback, failure: (error: GeolocationPositionError) => void) {
          failure({
            code: 2,
            message: "Location unavailable",
            PERMISSION_DENIED: 1,
            POSITION_UNAVAILABLE: 2,
            TIMEOUT: 3,
          });
          return 1;
        },
        clearWatch() {},
      },
    },
  });
  let received: DOMException | undefined;

  location.watchPosition(
    () => assert.fail("position received"),
    (error) => { received = error; },
  );

  assert.equal(received?.name, "NotReadableError");
});

void test("uses the native bridge and cancels watched updates", () => {
  const native = connectNative();
  const updates: number[] = [];

  const cancel = location.watchPosition(
    (position) => updates.push(position.coords.latitude),
    () => assert.fail("watch failed"),
  );
  const { session, id } = native.sent[1] ?? {};
  native.respond({ value: nativePosition(52, 0.2) }, false);
  cancel();

  assert.deepEqual(updates, [52]);
  assert.deepEqual(native.sent[2], { type: "cancel", session, id });
});

void test("ignores native responses from an earlier page session", async () => {
  const native = connectNative();

  const position = location.getCurrentPosition();
  native.deliver({ session: "earlier-page", id: native.sent[1]?.id, value: nativePosition(1, 2), done: true });
  native.respond({ value: nativePosition(52, 0.2) });

  assert.equal((await position).coords.latitude, 52);
});

void test("preserves native DOM exception errors", async () => {
  const native = connectNative();

  const position = location.getCurrentPosition();
  native.respond({ error: { name: "NotAllowedError", message: "Location permission was denied" } });

  await assert.rejects(
    position,
    (error: unknown) =>
      error instanceof DOMException &&
      error.name === "NotAllowedError" &&
      error.message === "Location permission was denied",
  );
});

void test("keeps a native watch active after an error", () => {
  const native = connectNative();
  const updates: number[] = [];
  const errors: string[] = [];

  const cancel = location.watchPosition(
    (position) => updates.push(position.coords.latitude),
    (error) => errors.push(error.name),
  );
  native.respond({ error: { name: "NotReadableError", message: "Location is unavailable" } }, false);
  native.respond({ value: nativePosition(52, 0.2) }, false);
  cancel();

  assert.deepEqual(errors, ["NotReadableError"]);
  assert.deepEqual(updates, [52]);
});

function browserPosition(latitude: number, longitude: number): GeolocationPosition {
  return nativePosition(latitude, longitude) as GeolocationPosition;
}

function nativePosition(latitude: number, longitude: number) {
  return {
    coords: {
      latitude,
      longitude,
      accuracy: 1,
      altitude: null,
      altitudeAccuracy: null,
      heading: null,
      speed: null,
    },
    timestamp: 1,
  };
}
