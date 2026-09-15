import { writeStderr, writeStdout } from "tokamak:host";
import { format, inspect } from "../node/util.mjs";

function line(values) {
  return (values.length === 0 ? "" : format(...values)) + "\n";
}

const counts = new Map();
const times = new Map();

export const console = {
  log(...values) { writeStdout(line(values)); },
  info(...values) { writeStdout(line(values)); },
  debug(...values) { writeStdout(line(values)); },
  warn(...values) { writeStderr(line(values)); },
  error(...values) { writeStderr(line(values)); },
  trace(...values) {
    const stack = new Error().stack?.split("\n").slice(1).join("\n") ?? "";
    writeStderr("Trace" + (values.length ? ": " + format(...values) : "") + "\n" + stack);
  },
  dir(value, options) { writeStdout(inspect(value, options) + "\n"); },
  assert(condition, ...values) {
    if (condition) return;
    console.error("Assertion failed" + (values.length ? ": " + format(...values) : ""));
  },
  count(label = "default") {
    const next = (counts.get(label) ?? 0) + 1;
    counts.set(label, next);
    console.log(`${label}: ${next}`);
  },
  countReset(label = "default") { counts.delete(label); },
  time(label = "default") { times.set(label, Date.now()); },
  timeLog(label = "default", ...values) {
    const start = times.get(label);
    if (start !== undefined) console.log(`${label}: ${Date.now() - start}ms`, ...values);
  },
  timeEnd(label = "default") {
    const start = times.get(label);
    times.delete(label);
    if (start !== undefined) console.log(`${label}: ${Date.now() - start}ms`);
  },
  group(...values) { if (values.length) console.log(...values); },
  groupCollapsed(...values) { console.group(...values); },
  groupEnd() {},
  table(value) { console.log(value); },
  clear() {},
};

export function installConsoleGlobal() {
  globalThis.console = console;
}
