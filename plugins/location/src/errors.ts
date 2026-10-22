/** The errors location operations reject with. */

export function permissionDenied(): DOMException {
  return new DOMException("Location permission was denied", "NotAllowedError");
}

export function backgroundDenied(): DOMException {
  return new DOMException("Location permission does not cover the background", "NotAllowedError");
}

export function timedOut(): DOMException {
  return new DOMException("Location was not found in time", "TimeoutError");
}

export function unavailable(message: string): DOMException {
  return new DOMException(message, "NotReadableError");
}
