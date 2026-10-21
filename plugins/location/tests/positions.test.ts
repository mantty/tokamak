import assert from "node:assert/strict";
import { setImmediate } from "node:timers/promises";
import { test } from "node:test";

import type { NativePlatform } from "@tokamakdev/plugin";
import { decodeValues } from "@tokamakdev/plugin/streams";
import { connectNative, type Answer, type FakeNativeTransport, type Received } from "@tokamakdev/plugin/testing";

import { location } from "../src/index.js";

type Answers = Record<string, Answer | ((request: Received) => Answer | undefined)>;

const object = (id: string, ...types: string[]) => ({ "#": "object", id, types });

/** A device on `platform` that answers the plugin's calls from `answers`, by `plugin.method`. */
function device(platform: NativePlatform, answers: Answers): FakeNativeTransport {
  const native = connectNative(platform);
  const common: Answers = {
    "location.servicesEnabled": { value: true },
    "location.permission": { value: { inUse: "granted", background: "prompt", precision: "precise" } },
    "tokamak.resource.scope": { value: "scope", done: false },
    "tokamak.resource.release": { value: null },
  };
  native.answer((request) => {
    const answer = answers[`${request.plugin ?? ""}.${request.method ?? ""}`] ?? common[`${request.plugin ?? ""}.${request.method ?? ""}`];
    return typeof answer === "function" ? answer(request) : answer;
  });
  return native;
}

const target = (request: Received) => ((request.arguments as { target?: { id: string } }).target?.id);

/** Core Location's answers about fixes, each taken `age` milliseconds ago: `old` at latitude 1, others at 52. */
function appleFixes(age: number): Answers {
  return {
    "location/apple.CLLocation.timestamp": { value: { "#": "date", value: Date.now() - age } },
    "location/apple.CLLocation.coordinate": (request) => ({ value: { latitude: target(request) === "old" ? 1 : 52, longitude: 0.2 } }),
    "location/apple.CLLocation.horizontalAccuracy": { value: 5 },
    "location/apple.CLLocation.altitude": { value: 30 },
    "location/apple.CLLocation.verticalAccuracy": { value: -1 },
    "location/apple.CLLocation.course": { value: 90 },
    "location/apple.CLLocation.speed": { value: -1 },
  };
}

/** Waits for `condition`, failing after a thousand turns of the event loop. */
async function until(condition: () => boolean): Promise<void> {
  for (let turn = 0; !condition(); turn++) {
    if (turn === 1000) assert.fail("The plugin did not make the expected call");
    await setImmediate();
  }
}

const calls = (native: FakeNativeTransport, method: string) => native.sent.filter((request) => request.method === method);

void test("Apple answers with Core Location's recent fix, as the Geolocation API reports one", async () => {
  const native = device("ios", {
    ...appleFixes(500),
    "location/apple.CLLocationManager.init()": { value: object("manager", "CLLocationManager") },
    "location/apple.CLLocationManager.location": { value: object("recent", "CLLocation") },
  });
  const position = await location(native.ctx).getCurrentPosition({ maximumAge: 1000 });
  assert.deepEqual(position.coords, { latitude: 52, longitude: 0.2, accuracy: 5, altitude: null, altitudeAccuracy: null, heading: 90, speed: null });
  assert.equal(calls(native, "CLLocationManager.requestLocation()").length, 0);
});

void test("Apple takes a new fix, asking again while Core Location does not know the position", async () => {
  let delegate = 0;
  const native = device("ios", {
    ...appleFixes(1000),
    "location/apple.CLLocationManager.init()": { value: object("manager", "CLLocationManager") },
    "location/apple.CLLocationManager.location": { value: object("old", "CLLocation") },
    "location/apple.CLLocationManagerDelegate": (request) => {
      delegate = request.id;
      return { value: "delegate", done: false };
    },
    "location/apple.CLLocationManager.delegate=": { value: null },
    "location/apple.CLLocationManager.requestLocation()": { value: null },
  });
  const pending = location(native.ctx).getCurrentPosition({ maximumAge: 100 });
  await until(() => calls(native, "CLLocationManager.requestLocation()").length === 1);
  const unknown = { "#": "error", name: "OperationError", message: "Unknown", cause: { domain: "kCLErrorDomain", code: 0 } };
  native.deliver(delegate, { value: { method: "locationManagerDidFailWithError", arguments: [object("manager", "CLLocationManager"), unknown] }, done: false });
  await until(() => calls(native, "CLLocationManager.requestLocation()").length === 2);
  native.deliver(delegate, { value: { method: "locationManagerDidUpdateLocations", arguments: [object("manager", "CLLocationManager"), [object("new", "CLLocation")]] }, done: false });
  assert.equal((await pending).coords.latitude, 52);
  assert.ok(native.sent.some((request) => request.type === "cancel"), "the delegate is released");
});

void test("location asks for in-use permission it lacks, and rejects without it", async () => {
  const native = device("ios", {
    "location.permission": { value: { inUse: "prompt", background: "prompt", precision: "precise" } },
    "location.requestPermission": { value: { inUse: "denied", background: "denied", precision: "precise" } },
  });
  await assert.rejects(location(native.ctx).getCurrentPosition(), { name: "NotAllowedError" });
  assert.deepEqual(calls(native, "requestPermission").length, 1);
  assert.equal(calls(native, "CLLocationManager.init()").length, 0);

  const off = device("android", { "location.servicesEnabled": { value: false } });
  await assert.rejects(location(off.ctx).getCurrentPosition(), { name: "NotReadableError", message: "Location services are disabled" });
});

void test("Apple watches with updates until the stream is cancelled", async () => {
  let delegate = 0;
  const native = device("macos", {
    ...appleFixes(0),
    "location/apple.CLLocationManager.init()": { value: object("manager", "CLLocationManager") },
    "location/apple.CLLocationManagerDelegate": (request) => {
      delegate = request.id;
      return { value: "delegate", done: false };
    },
    "location/apple.CLLocationManager.delegate=": { value: null },
    "location/apple.CLLocationManager.startUpdatingLocation()": { value: null },
    "location/apple.CLLocationManager.stopUpdatingLocation()": { value: null },
  });
  const values = decodeValues<{ coords: { latitude: number } }>(location(native.ctx).watchPosition());
  const first = values.next();
  await until(() => calls(native, "CLLocationManager.startUpdatingLocation()").length === 1);
  native.deliver(delegate, { value: { method: "locationManagerDidUpdateLocations", arguments: [object("manager", "CLLocationManager"), [object("new", "CLLocation")]] }, done: false });
  assert.equal((await first).value?.coords.latitude, 52);
  await values.return(undefined);
  await until(() => calls(native, "CLLocationManager.stopUpdatingLocation()").length === 1);
});

/** Android's answers about the location manager, whose providers are `providers`. */
function android(providers: Record<string, boolean>, answers: Answers): FakeNativeTransport {
  return device("android", {
    "tokamak.resource.root": { value: "context" },
    "location/android.Context#getSystemService(Ljava/lang/String;)Ljava/lang/Object;": { value: object("manager", "android.location.LocationManager") },
    "location/android.LocationManager#isProviderEnabled(Ljava/lang/String;)Z": (request) => {
      const [name] = (request.arguments as { arguments: [string] }).arguments;
      return { value: providers[name] ?? false };
    },
    "location/android.Context#getMainExecutor()Ljava/util/concurrent/Executor;": { value: object("executor", "java.util.concurrent.Executor") },
    "location/android.CancellationSignal#<init>()V": { value: object("signal", "android.os.CancellationSignal") },
    "location/android.CancellationSignal#cancel()V": { value: null },
    ...answers,
  });
}

const androidFix: Answers = Object.fromEntries(Object.entries({
  "getLatitude()D": 48, "getLongitude()D": 2, "getAccuracy()F": 3, "hasAltitude()Z": true, "getAltitude()D": 35,
  "hasVerticalAccuracy()Z": false, "getVerticalAccuracyMeters()F": 0, "hasBearing()Z": false, "getBearing()F": 0,
  "hasSpeed()Z": true, "getSpeed()F": 1.5, "getTime()J": 1000,
}).map(([member, value]) => [`location/android.Location#${member}`, { value }]));

void test("Android takes a fix from the first enabled provider", async () => {
  let consumer = 0;
  const native = android({ network: true }, {
    ...androidFix,
    "location/android.LocationManager#getLastKnownLocation(Ljava/lang/String;)Landroid/location/Location;": { value: null },
    "location/android.Consumer": (request) => {
      consumer = request.id;
      return { value: "consumer", done: false };
    },
    "location/android.LocationManager#getCurrentLocation(Ljava/lang/String;Landroid/os/CancellationSignal;Ljava/util/concurrent/Executor;Ljava/util/function/Consumer;)V": (request) => {
      assert.equal((request.arguments as { arguments: unknown[] }).arguments[0], "network");
      queueMicrotask(() => { native.deliver(consumer, { value: { method: "accept", arguments: [object("fix", "android.location.Location")] }, done: false }); });
      return { value: null };
    },
  });
  const position = await location(native.ctx).getCurrentPosition();
  assert.deepEqual(position, { coords: { latitude: 48, longitude: 2, accuracy: 3, altitude: 35, altitudeAccuracy: null, heading: null, speed: 1.5 }, timestamp: 1000 });
});

void test("Android without a fix in the background reports missing background permission", async () => {
  let consumer = 0;
  const native = android({ gps: true }, {
    "tokamak.lifecycleStage": { value: "background" },
    "location/android.LocationManager#getLastKnownLocation(Ljava/lang/String;)Landroid/location/Location;": { value: null },
    "location/android.Consumer": (request) => {
      consumer = request.id;
      return { value: "consumer", done: false };
    },
    "location/android.LocationManager#getCurrentLocation(Ljava/lang/String;Landroid/os/CancellationSignal;Ljava/util/concurrent/Executor;Ljava/util/function/Consumer;)V": () => {
      queueMicrotask(() => { native.deliver(consumer, { value: { method: "accept", arguments: [null] }, done: false }); });
      return { value: null };
    },
  });
  await assert.rejects(location(native.ctx).getCurrentPosition(), { name: "NotAllowedError", message: /background/ });
});

void test("Android rejects with TimeoutError when no fix comes in time, cancelling the request", async () => {
  const native = android({ gps: true }, {
    "location/android.LocationManager#getLastKnownLocation(Ljava/lang/String;)Landroid/location/Location;": { value: null },
    "location/android.Consumer": { value: "consumer", done: false },
    "location/android.LocationManager#getCurrentLocation(Ljava/lang/String;Landroid/os/CancellationSignal;Ljava/util/concurrent/Executor;Ljava/util/function/Consumer;)V": { value: null },
  });
  await assert.rejects(location(native.ctx).getCurrentPosition({ timeout: 10 }), { name: "TimeoutError" });
  await until(() => calls(native, "CancellationSignal#cancel()V").length === 1);
});
