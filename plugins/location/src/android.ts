import type { TokamakContext } from "@tokamakdev/plugin";
import { applicationContext, release } from "@tokamakdev/plugin/bindings";

import {
  CancellationSignal,
  Consumer,
  Context,
  Location,
  LocationListener,
  LocationManager,
  SystemClock,
  type Executor,
} from "../android/bindings.js";
import { permissionDenied, unavailable, within } from "./errors.js";
import type { Position, PositionOptions } from "./types.js";

/**
 * The device's position: the provider's last known when no older than `maximumAge`, or a new fix
 * within `timeout`. Null when Android finds none.
 */
export async function currentPosition(ctx: TokamakContext, { maximumAge = 0, timeout }: PositionOptions): Promise<Position | null> {
  const { context, manager } = await locationManager(ctx);
  const name = await provider(manager);
  const recent = await manager.getLastKnownLocation(name);
  if (recent && await age(ctx, recent) <= maximumAge) return await position(recent);
  const [signal, executor] = await Promise.all([CancellationSignal.new(ctx), mainExecutor(context)]);
  let consumer: Consumer | undefined;
  let finished = false;
  const found = new Promise<Position | null>((resolve, reject) => {
    Consumer.implement(ctx, {
      accept(location) { resolve(location instanceof Location ? position(location) : null); },
    }).then(async (created) => {
      consumer = created;
      if (finished) {
        release(created);
        return;
      }
      await manager.getCurrentLocation(name, signal, executor, created);
    }).catch((failure: unknown) => { reject(locationError(failure as Error)); });
  });
  return within(found, timeout, () => {
    finished = true;
    void signal.cancel();
    if (consumer) release(consumer);
  });
}

/** Passes each position the provider reports to `next`, and its failures to `error`, until the returned function is called. */
export async function watchPositions(
  ctx: TokamakContext,
  next: (position: Position) => void,
  error: (error: unknown) => void,
): Promise<() => void> {
  const { context, manager } = await locationManager(ctx);
  const [name, executor] = await Promise.all([provider(manager), mainExecutor(context)]);
  const listener = await LocationListener.implement(ctx, {
    async onLocationChanged(location) {
      next(await position(location));
      release(location);
    },
    onProviderDisabled() { error(unavailable("Location provider is unavailable")); },
  });
  try {
    await manager.requestLocationUpdates(name, 1000, 0, executor, listener);
  } catch (failure) {
    release(listener);
    throw locationError(failure as Error);
  }
  return () => {
    void manager.removeUpdates(listener).finally(() => { release(listener); });
  };
}

async function locationManager(ctx: TokamakContext): Promise<{ context: Context; manager: LocationManager }> {
  const context = new Context(await applicationContext(ctx));
  const manager = await context.getSystemService(Context.LOCATION_SERVICE);
  if (!(manager instanceof LocationManager)) throw unavailable("The device has no location service");
  return { context, manager };
}

async function mainExecutor(context: Context): Promise<Executor> {
  const executor = await context.getMainExecutor();
  if (!executor) throw unavailable("The app has no main executor");
  return executor;
}

/** The first enabled of the GPS, network and fused providers, or any other enabled provider. */
async function provider(manager: LocationManager): Promise<string> {
  for (const name of [LocationManager.GPS_PROVIDER, LocationManager.NETWORK_PROVIDER, LocationManager.FUSED_PROVIDER]) {
    if (await manager.isProviderEnabled(name)) return name;
  }
  const [other] = await manager.getProviders(true);
  if (!other) throw unavailable("No location provider is available");
  return other;
}

/** The milliseconds since Android took `location`. */
async function age(ctx: TokamakContext, location: Location): Promise<number> {
  const [now, taken] = await Promise.all([SystemClock.elapsedRealtimeNanos(ctx), location.getElapsedRealtimeNanos()]);
  return (now - taken) / 1_000_000;
}

async function position(location: Location): Promise<Position> {
  const [latitude, longitude, accuracy, hasAltitude, altitude, hasVertical, vertical, hasBearing, bearing, hasSpeed, speed, time] =
    await Promise.all([
      location.getLatitude(),
      location.getLongitude(),
      location.getAccuracy(),
      location.hasAltitude(),
      location.getAltitude(),
      location.hasVerticalAccuracy(),
      location.getVerticalAccuracyMeters(),
      location.hasBearing(),
      location.getBearing(),
      location.hasSpeed(),
      location.getSpeed(),
      location.getTime(),
    ]);
  return {
    coords: {
      latitude,
      longitude,
      accuracy,
      altitude: hasAltitude ? altitude : null,
      altitudeAccuracy: hasVertical ? vertical : null,
      heading: hasBearing ? bearing : null,
      speed: hasSpeed ? speed : null,
    },
    timestamp: time,
  };
}

/** The error a failed Android call rejected with, as location operations name it. */
function locationError(failure: Error): Error {
  if (!(failure instanceof DOMException)) return failure;
  const exception = (failure.cause as { exception?: string } | undefined)?.exception;
  return exception === "java.lang.SecurityException" ? permissionDenied() : unavailable(failure.message);
}
