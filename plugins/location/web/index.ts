import type { Position, PositionCallback, PositionErrorCallback } from "../src/index.js";

export function getCurrentPosition(): Promise<Position> {
  return new Promise((resolve, reject) => {
    const browserLocation = browserLocationApi();
    browserLocation.getCurrentPosition(
      (position) => {
        resolve(copyPosition(position));
      },
      (error) => {
        reject(positionError(error));
      },
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

function browserLocationApi(): Geolocation {
  const root: { navigator?: { geolocation?: Geolocation } } = globalThis;
  const browserLocation = root.navigator?.geolocation;
  if (browserLocation) return browserLocation;
  throw new DOMException("Location is unavailable", "NotSupportedError");
}

function copyPosition(position: GeolocationPosition): Position {
  const { coords } = position;
  return {
    coords: {
      latitude: coords.latitude,
      longitude: coords.longitude,
      accuracy: coords.accuracy,
      altitude: coords.altitude,
      altitudeAccuracy: coords.altitudeAccuracy,
      heading: coords.heading,
      speed: coords.speed,
    },
    timestamp: position.timestamp,
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
