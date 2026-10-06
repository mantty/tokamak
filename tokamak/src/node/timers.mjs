import * as promises from "./timers-promises.mjs";
import { clearImmediate, clearInterval, clearTimeout, setImmediate, setInterval, setTimeout } from "../globals/timers.mjs";

export function active(timer) { return timer; }
export function enroll() { throw new Error("Not implemented. Please use setTimeout() instead."); }
export function unenroll() {}

export { clearImmediate, clearInterval, clearTimeout, promises, setImmediate, setInterval, setTimeout };
export default { active, clearImmediate, clearInterval, clearTimeout, enroll, promises, setImmediate, setInterval, setTimeout, unenroll };
