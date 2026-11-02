import { asyncContextGet, asyncContextSet } from "tokamak:host";

declare global {
  // The value each AsyncLocalStorage has in one async context, which the
  // engine carries across promise jobs and the host across its callbacks.
  type AsyncContextFrame = ReadonlyMap<object | symbol, unknown>;
}

export const captureAsyncContext = asyncContextGet;

export function runInAsyncContext<This, Args extends unknown[], Result>(
  context: AsyncContextFrame | undefined,
  callback: (this: This, ...args: Args) => Result,
  receiver: This,
  args: Args,
): Result {
  const previous = asyncContextGet();
  asyncContextSet(context);
  try { return Reflect.apply(callback, receiver, args); }
  finally { asyncContextSet(previous); }
}
