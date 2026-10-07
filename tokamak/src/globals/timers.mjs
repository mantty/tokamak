import { scheduleTimer } from "tokamak:host";
import { captureAsyncContext, runInAsyncContext } from "../builtins/async-context.mjs";
import { logUncaught, reportError } from "../events/web.mjs";

const queueEngineMicrotask = globalThis.queueMicrotask;
const timers = new Map();
let nextTimer = 1;

function timer(callback, timeout, repeat, args) {
  if (typeof callback !== "function") throw new TypeError("Timer callback must be a function");
  const id = nextTimer++;
  const milliseconds = Math.max(0, Math.min(2 ** 32 - 1, Number(timeout) || 0));
  const context = captureAsyncContext();
  const fire = () => {
    if (repeat) timers.set(id, scheduleTimer(fire, Math.max(1, milliseconds)));
    else timers.delete(id);
    try { runInAsyncContext(context, callback, undefined, args); }
    catch (error) { logUncaught(error); }
  };
  timers.set(id, scheduleTimer(fire, milliseconds));
  return id;
}

function clearTimer(id) {
  timers.get(id)?.();
  timers.delete(id);
}

export function setTimeout(callback, timeout, ...args) { return timer(callback, timeout, false, args); }
export function setInterval(callback, timeout, ...args) { return timer(callback, timeout, true, args); }
export function setImmediate(callback, ...args) { return timer(callback, 0, false, args); }
export function clearTimeout(id) { clearTimer(id); }
export function clearInterval(id) { clearTimer(id); }
export function clearImmediate(id) { clearTimer(id); }

export function queueMicrotask(callback) {
  if (typeof callback !== "function") {
    throw new TypeError("Failed to execute 'queueMicrotask' on 'ServiceWorkerGlobalScope': parameter 1 is not of type 'function'.");
  }
  queueEngineMicrotask(() => {
    try { callback(); }
    catch (error) { reportError(error); }
  });
}
