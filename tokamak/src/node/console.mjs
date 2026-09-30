import { writeStderr, writeStdout } from "tokamak:host";
import { console as consoleObject } from "../globals/console.mjs";

function noop() {}

const methods = [
  "assert", "clear", "context", "count", "countReset", "createTask", "debug", "dir", "dirxml", "error", "group",
  "groupCollapsed", "groupEnd", "info", "log", "profile", "profileEnd", "table", "time", "timeEnd", "timeLog",
  "timeStamp", "trace", "warn",
];
for (const name of methods) if (typeof consoleObject[name] !== "function") consoleObject[name] = noop;
consoleObject._ignoreErrors ??= true;
consoleObject._stderr ??= { write(text) { writeStderr(String(text)); return true; } };
consoleObject._stdout ??= { write(text) { writeStdout(String(text)); return true; } };
consoleObject._stderrErrorHandler ??= noop;
consoleObject._stdoutErrorHandler ??= noop;
consoleObject._times ??= new Map();

export class Console {
  constructor() {
    throw Object.assign(new Error("The Console method is not implemented"), { code: "ERR_METHOD_NOT_IMPLEMENTED" });
  }
}

consoleObject.Console = Console;

export const {
  assert, clear, context, count, countReset, createTask, debug, dir, dirxml, error, group, groupCollapsed, groupEnd, info,
  log, profile, profileEnd, table, time, timeEnd, timeLog, timeStamp, trace, warn, _ignoreErrors, _stderr,
  _stderrErrorHandler, _stdout, _stdoutErrorHandler, _times,
} = consoleObject;

export default consoleObject;
