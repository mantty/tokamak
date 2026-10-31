import * as timers from "../globals/timers.ts";
import { objectOf } from "../globals/objects.ts";
import { abortError } from "./errors.ts";
import { validateAbortSignal, type AbortSignalLike } from "./validators.ts";

// The abort signal in a timer's `options`, validated as Node validates it.
function timerSignal(options: unknown): AbortSignalLike | undefined {
  if (options === null || (options !== undefined && typeof options !== "object")) throw new TypeError("The options argument must be an object");
  const signal = options === undefined ? undefined : objectOf(options).signal;
  validateAbortSignal(signal, "options.signal");
  return signal;
}

function timerPromise<T>(delay: unknown = 0, value: T, options: unknown): Promise<T> {
  return new Promise((resolve, reject) => {
    const signal = timerSignal(options);
    if (signal?.aborted) { reject(abortError()); return; }
    const id = timers.setTimeout(() => resolve(value), delay);
    signal?.addEventListener("abort", () => { timers.clearTimeout(id); reject(abortError()); }, { once: true });
  });
}

export function setTimeout<T>(delay?: unknown, value?: T, options?: unknown): Promise<T | undefined> { return timerPromise(delay, value, options); }
export function setImmediate<T>(value?: T, options?: unknown): Promise<T | undefined> { return timerPromise(0, value, options); }

export async function* setInterval<T>(delay: unknown = 0, value?: T, options?: unknown): AsyncGenerator<T | undefined> {
  const signal = timerSignal(options);
  let stopped = false;
  // Ticks that fired while no iteration was waiting.
  let missed = 0;
  let next: PromiseWithResolvers<T | undefined> | undefined;
  const id = timers.setInterval(() => {
    if (next) { next.resolve(value); next = undefined; }
    else missed += 1;
  }, delay);
  const abort = () => {
    stopped = true;
    timers.clearInterval(id);
    next?.reject(abortError());
    next = undefined;
  };
  signal?.addEventListener("abort", abort, { once: true });
  try {
    while (!stopped) {
      if (missed > 0) { missed -= 1; yield value; continue; }
      next = Promise.withResolvers();
      yield await next.promise;
    }
  } finally {
    timers.clearInterval(id);
    signal?.removeEventListener("abort", abort);
  }
}

export const scheduler = { wait: (delay?: unknown, options?: unknown) => timerPromise(delay, undefined, options) };

export default { scheduler, setImmediate, setInterval, setTimeout };
