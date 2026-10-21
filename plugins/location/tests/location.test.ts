import assert from "node:assert/strict";
import { afterEach, test } from "node:test";

import { decodeValues } from "@tokamakdev/plugin/streams";

import { location } from "../web/client.js";

const originalNavigator = Object.getOwnPropertyDescriptor(globalThis, "navigator");

afterEach(() => {
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

void test("passes position options to browser location", async () => {
  let received: PositionOptions | undefined;
  Object.defineProperty(globalThis, "navigator", {
    configurable: true,
    value: {
      geolocation: {
        getCurrentPosition(success: PositionCallback, _: unknown, options: PositionOptions) {
          received = options;
          success(browserPosition(51.5, -0.1));
        },
      },
    },
  });

  await location.getCurrentPosition({ maximumAge: 1000, timeout: 5000 });

  assert.deepEqual(received, { maximumAge: 1000, timeout: 5000 });
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

void test("forwards browser watch failures and clears the watch", async () => {
  let cleared = 0;
  Object.defineProperty(globalThis, "navigator", { configurable: true, value: { geolocation: {
    watchPosition(_: unknown, failure: (error: unknown) => void) {
      failure({ code: 2, message: "Unavailable", POSITION_UNAVAILABLE: 2 });
      return 7;
    },
    clearWatch(id: number) { cleared = id; },
  } } });
  await assert.rejects(decodeValues(await location.watchPosition()).next(), { name: "NotReadableError" });
  assert.equal(cleared, 7);
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
