import * as promises from "./timers-promises.mjs";

export const setTimeout = (...args) => globalThis.setTimeout(...args);
export const clearTimeout = (...args) => globalThis.clearTimeout(...args);
export const setInterval = (...args) => globalThis.setInterval(...args);
export const clearInterval = (...args) => globalThis.clearInterval(...args);
export const setImmediate = (...args) => globalThis.setImmediate(...args);
export const clearImmediate = (...args) => globalThis.clearImmediate(...args);

export function active(timer) { return timer; }
export function enroll() { throw new Error("Not implemented. Please use setTimeout() instead."); }
export function unenroll() {}

export { promises };
export default { active, clearImmediate, clearInterval, clearTimeout, enroll, promises, setImmediate, setInterval, setTimeout, unenroll };
