import type {
  LocationPermission,
  Position,
  PositionCallback,
  PositionErrorCallback,
  PositionOptions,
} from "../src/types.js";

export function getCurrentPosition(options: PositionOptions): Promise<Position> {
  return new Promise((resolve, reject) => {
    const browserLocation = browserLocationApi();
    browserLocation.getCurrentPosition(
      (position) => {
        resolve(copyPosition(position));
      },
      (error) => {
        reject(positionError(error));
      },
      options,
    );
  });
}

export function watchPosition(
  next: PositionCallback,
  error: PositionErrorCallback,
): () => void {
  const browserLocation = browserLocationApi();
  const id = browserLocation.watchPosition(
    (position) => {
      next(copyPosition(position));
    },
    (failure) => {
      error(positionError(failure));
    },
  );
  return () => {
    browserLocation.clearWatch(id);
  };
}

/** The page's location permission, which the web has only for pages in use. */
export async function permission(): Promise<LocationPermission> {
  const permissions = (globalThis as { navigator?: { permissions?: Permissions } }).navigator?.permissions;
  if (!permissions) throw new DOMException("The Permissions API is unavailable", "NotSupportedError");
  const { state } = await permissions.query({ name: "geolocation" });
  return { inUse: state, backgroundAlways: "denied", precision: "precise" };
}

/** Rejects as the web does for what only native apps have. */
export function unsupported(): Promise<never> {
  return Promise.reject(new DOMException("Native location is unavailable", "NotSupportedError"));
}

function browserLocationApi(): Geolocation {
  const root: { navigator?: { geolocation?: Geolocation } } = globalThis;
  const browserLocation = root.navigator?.geolocation;
  if (browserLocation) return browserLocation;
  throw new DOMException("Location is unavailable", "NotSupportedError");
}

/** `position` as native platforms give it: without the coordinates the device does not report. */
function copyPosition({ coords, timestamp }: GeolocationPosition): Position {
  const { latitude, longitude, accuracy, altitude, altitudeAccuracy, heading, speed } = coords;
  return {
    coords: {
      latitude,
      longitude,
      accuracy,
      ...(altitude === null ? {} : { altitude }),
      ...(altitudeAccuracy === null ? {} : { altitudeAccuracy }),
      ...(heading === null ? {} : { heading }),
      ...(speed === null ? {} : { speed }),
    },
    timestamp,
  };
}

function positionError(error: GeolocationPositionError): DOMException {
  const name = {
    [error.PERMISSION_DENIED]: "NotAllowedError",
    [error.POSITION_UNAVAILABLE]: "NotReadableError",
    [error.TIMEOUT]: "TimeoutError",
  }[error.code];
  return new DOMException(error.message, name ?? "UnknownError");
}
