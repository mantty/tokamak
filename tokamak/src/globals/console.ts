import { writeStderr, writeStdout } from "tokamak:host";
import { format, inspect } from "../node/util.ts";

function line(values: unknown[]): string {
  return (values.length === 0 ? "" : format(...values)) + "\n";
}

const counts = new Map<unknown, number>();
const times = new Map<unknown, number>();

export const console = {
  log(...values: unknown[]): void { writeStdout(line(values)); },
  info(...values: unknown[]): void { writeStdout(line(values)); },
  debug(...values: unknown[]): void { writeStdout(line(values)); },
  warn(...values: unknown[]): void { writeStderr(line(values)); },
  error(...values: unknown[]): void { writeStderr(line(values)); },
  trace(...values: unknown[]): void {
    const stack = new Error().stack?.split("\n").slice(1).join("\n") ?? "";
    writeStderr("Trace" + (values.length ? ": " + format(...values) : "") + "\n" + stack);
  },
  dir(value: unknown, options?: unknown): void { writeStdout(inspect(value, options) + "\n"); },
  dirxml(...values: unknown[]): void { console.log(...values); },
  assert(condition?: unknown, ...values: unknown[]): void {
    if (condition) return;
    console.error("Assertion failed" + (values.length ? ": " + format(...values) : ""));
  },
  count(label: unknown = "default"): void {
    const next = (counts.get(label) ?? 0) + 1;
    counts.set(label, next);
    console.log(`${label}: ${next}`);
  },
  countReset(label: unknown = "default"): void { counts.delete(label); },
  time(label: unknown = "default"): void { times.set(label, Date.now()); },
  timeLog(label: unknown = "default", ...values: unknown[]): void {
    const start = times.get(label);
    if (start !== undefined) console.log(`${label}: ${Date.now() - start}ms`, ...values);
  },
  timeEnd(label: unknown = "default"): void {
    const start = times.get(label);
    times.delete(label);
    if (start !== undefined) console.log(`${label}: ${Date.now() - start}ms`);
  },
  group(...values: unknown[]): void { if (values.length) console.log(...values); },
  groupCollapsed(...values: unknown[]): void { console.group(...values); },
  groupEnd() {},
  table(value: unknown): void { console.log(value); },
  clear() {},
  context() { return console; },
  profile() {},
  profileEnd() {},
  timeStamp() {},
};
