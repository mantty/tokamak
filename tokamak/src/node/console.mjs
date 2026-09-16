import { writeStderr, writeStdout } from "tokamak:host";
import { console as webConsole } from "../globals/console.mjs";

function noop() {}

function createConsole() {
  const value = webConsole;
  const methods = [
    "assert", "clear", "context", "count", "countReset", "createTask", "debug", "dir", "dirxml", "error", "group",
    "groupCollapsed", "groupEnd", "info", "log", "profile", "profileEnd", "table", "time", "timeEnd", "timeLog",
    "timeStamp", "trace", "warn",
  ];
  for (const name of methods) if (typeof value[name] !== "function") value[name] = noop;
  value._ignoreErrors ??= true;
  value._stderr ??= { write(text) { writeStderr(String(text)); return true; } };
  value._stdout ??= { write(text) { writeStdout(String(text)); return true; } };
  value._stderrErrorHandler ??= noop;
  value._stdoutErrorHandler ??= noop;
  value._times ??= new Map();
  return value;
}

const consoleObject = createConsole();

export class Console {
  constructor() {
    const error = new Error("The Console method is not implemented");
    error.code = "ERR_METHOD_NOT_IMPLEMENTED";
    throw error;
  }
}

consoleObject.Console = Console;

export const assert = consoleObject.assert;
export const clear = consoleObject.clear;
export const context = consoleObject.context;
export const count = consoleObject.count;
export const countReset = consoleObject.countReset;
export const createTask = consoleObject.createTask;
export const debug = consoleObject.debug;
export const dir = consoleObject.dir;
export const dirxml = consoleObject.dirxml;
export const error = consoleObject.error;
export const group = consoleObject.group;
export const groupCollapsed = consoleObject.groupCollapsed;
export const groupEnd = consoleObject.groupEnd;
export const info = consoleObject.info;
export const log = consoleObject.log;
export const profile = consoleObject.profile;
export const profileEnd = consoleObject.profileEnd;
export const table = consoleObject.table;
export const time = consoleObject.time;
export const timeEnd = consoleObject.timeEnd;
export const timeLog = consoleObject.timeLog;
export const timeStamp = consoleObject.timeStamp;
export const trace = consoleObject.trace;
export const warn = consoleObject.warn;
export const _ignoreErrors = consoleObject._ignoreErrors;
export const _stderr = consoleObject._stderr;
export const _stderrErrorHandler = consoleObject._stderrErrorHandler;
export const _stdout = consoleObject._stdout;
export const _stdoutErrorHandler = consoleObject._stdoutErrorHandler;
export const _times = consoleObject._times;

export default consoleObject;
