import type { TokamakContext } from "@tokamakdev/plugin";
import { release } from "@tokamakdev/plugin/bindings";

import { CLLocationManager, CLLocationManagerDelegate, type CLLocation } from "../apple/bindings.js";
import { permissionDenied, unavailable, within } from "./errors.js";
import type { Position, PositionOptions } from "./types.js";

/** The Core Location error codes the plugin tells apart. */
const locationUnknown = 0;
const denied = 1;

/** The device's position: Core Location's latest when no older than `maximumAge`, or a new fix within `timeout`. */
export async function currentPosition(ctx: TokamakContext, { maximumAge = 0, timeout }: PositionOptions): Promise<Position> {
  const manager = await CLLocationManager.init(ctx);
  try {
    const recent = await manager.location();
    if (recent && Date.now() - (await recent.timestamp()).getTime() <= maximumAge) return await position(recent);
    return await nextFix(ctx, manager, timeout);
  } finally {
    release(manager);
  }
}

/** The next fix `manager` takes, asking again while Core Location does not know the position yet. */
function nextFix(ctx: TokamakContext, manager: CLLocationManager, timeout: number | undefined): Promise<Position> {
  let delegate: CLLocationManagerDelegate | undefined;
  let finished = false;
  const found = new Promise<Position>((resolve, reject) => {
    CLLocationManagerDelegate.implement(ctx, {
      locationManagerDidUpdateLocations(_, locations) {
        const latest = locations.at(-1);
        if (latest) resolve(position(latest));
      },
      locationManagerDidFailWithError(_, error) {
        if (code(error) === locationUnknown) manager.requestLocation().catch(reject);
        else reject(locationError(error));
      },
    }).then(async (created) => {
      delegate = created;
      if (finished) {
        release(created);
        return;
      }
      await manager.setDelegate(created);
      await manager.requestLocation();
    }).catch(reject);
  });
  return within(found, timeout, () => {
    finished = true;
    if (delegate) release(delegate);
  });
}

/** Passes each position Core Location reports to `next`, and its failures to `error`, until the returned function is called. */
export async function watchPositions(
  ctx: TokamakContext,
  next: (position: Position) => void,
  error: (error: unknown) => void,
): Promise<() => void> {
  const manager = await CLLocationManager.init(ctx);
  const delegate = await CLLocationManagerDelegate.implement(ctx, {
    async locationManagerDidUpdateLocations(updated, locations) {
      release(updated);
      for (const location of locations) {
        next(await position(location));
        release(location);
      }
    },
    locationManagerDidFailWithError(updated, failure) {
      release(updated);
      if (code(failure) !== locationUnknown) error(locationError(failure));
    },
  });
  await manager.setDelegate(delegate);
  await manager.startUpdatingLocation();
  return () => {
    void manager.stopUpdatingLocation().finally(() => {
      release(delegate);
      release(manager);
    });
  };
}

async function position(location: CLLocation): Promise<Position> {
  const [coordinate, accuracy, altitude, verticalAccuracy, course, speed, timestamp] = await Promise.all([
    location.coordinate(),
    location.horizontalAccuracy(),
    location.altitude(),
    location.verticalAccuracy(),
    location.course(),
    location.speed(),
    location.timestamp(),
  ]);
  // Core Location reports a negative accuracy, course or speed when it has none.
  return {
    coords: {
      latitude: coordinate.latitude,
      longitude: coordinate.longitude,
      accuracy,
      altitude: verticalAccuracy >= 0 ? altitude : null,
      altitudeAccuracy: verticalAccuracy >= 0 ? verticalAccuracy : null,
      heading: course >= 0 ? course : null,
      speed: speed >= 0 ? speed : null,
    },
    timestamp: timestamp.getTime(),
  };
}

/** The code of a Core Location error. */
function code(error: DOMException): number | undefined {
  const cause = error.cause as { domain?: string; code?: number } | undefined;
  return cause?.domain === "kCLErrorDomain" ? cause.code : undefined;
}

function locationError(error: DOMException): DOMException {
  return code(error) === denied ? permissionDenied() : unavailable(error.message);
}
