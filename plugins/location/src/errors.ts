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

/**
 * Rejects once `timeout` milliseconds pass without `found` settling, running `stop` either way. An
 * undefined `timeout` waits as long as it takes.
 */
export async function within<T>(found: Promise<T>, timeout: number | undefined, stop: () => void): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const expired = new Promise<never>((_, reject) => {
    if (timeout !== undefined) timer = setTimeout(() => { reject(timedOut()); }, timeout);
  });
  try {
    return await Promise.race([found, expired]);
  } finally {
    clearTimeout(timer);
    stop();
  }
}
