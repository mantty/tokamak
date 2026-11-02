import { scheduleTimer, type Cancellation } from "tokamak:host";
import { captureAsyncContext, runInAsyncContext } from "../builtins/async-context.ts";
import { logUncaught, reportError } from "../events/web.ts";
import { isFunction } from "./objects.ts";

const queueEngineMicrotask = globalThis.queueMicrotask;
const timers = new Map<number, Cancellation>();
let nextTimer = 1;

function timer(callback: unknown, timeout: unknown, repeat: boolean, args: unknown[]): number {
  if (!isFunction(callback)) throw new TypeError("Timer callback must be a function");
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

function clearTimer(id: unknown): void {
  if (typeof id !== "number") return;
  timers.get(id)?.cancel();
  timers.delete(id);
}

export function setTimeout(callback: unknown, timeout?: unknown, ...args: unknown[]): number { return timer(callback, timeout, false, args); }
export function setInterval(callback: unknown, timeout?: unknown, ...args: unknown[]): number { return timer(callback, timeout, true, args); }
export function setImmediate(callback: unknown, ...args: unknown[]): number { return timer(callback, 0, false, args); }
export function clearTimeout(id: unknown): void { clearTimer(id); }
export function clearInterval(id: unknown): void { clearTimer(id); }
export function clearImmediate(id: unknown): void { clearTimer(id); }

export function queueMicrotask(callback: unknown): void {
  if (typeof callback !== "function") {
    throw new TypeError("Failed to execute 'queueMicrotask' on 'ServiceWorkerGlobalScope': parameter 1 is not of type 'function'.");
  }
  queueEngineMicrotask(() => {
    try { callback(); }
    catch (error) { reportError(error); }
  });
}
