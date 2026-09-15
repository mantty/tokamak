import { asyncContextGet, asyncContextSet } from "tokamak:host";

export const captureAsyncContext = asyncContextGet;

export function runInAsyncContext(context, callback, receiver, args) {
  const previous = asyncContextGet();
  asyncContextSet(context);
  try { return Reflect.apply(callback, receiver, args); }
  finally { asyncContextSet(previous); }
}

export function bindAsyncContext(callback, context = asyncContextGet()) {
  return function (...args) { return runInAsyncContext(context, callback, this, args); };
}
