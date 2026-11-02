import { writeStderr, writeStdout } from "tokamak:host";
import { console as globalConsole } from "../globals/console.ts";
import { codedError } from "./errors.ts";

interface OutputStream {
  write(text: unknown): boolean;
}

// Members Node's console has beyond the global console's.
interface NodeConsoleMembers {
  createTask?: () => void;
  _ignoreErrors?: boolean;
  _stderr?: OutputStream;
  _stdout?: OutputStream;
  _stderrErrorHandler?: () => void;
  _stdoutErrorHandler?: () => void;
  _times?: Map<unknown, unknown>;
  Console?: typeof Console;
}

const consoleObject: typeof globalConsole & NodeConsoleMembers = globalConsole;

function noop() {}

// Loading `node:console` adds Node's members to the global console.
consoleObject.createTask ??= noop;
consoleObject._ignoreErrors ??= true;
consoleObject._stderr ??= { write(text) { writeStderr(String(text)); return true; } };
consoleObject._stdout ??= { write(text) { writeStdout(String(text)); return true; } };
consoleObject._stderrErrorHandler ??= noop;
consoleObject._stdoutErrorHandler ??= noop;
consoleObject._times ??= new Map();

export class Console {
  constructor() {
    throw codedError(Error, "The Console method is not implemented", "ERR_METHOD_NOT_IMPLEMENTED");
  }
}

consoleObject.Console = Console;

export const {
  assert, clear, context, count, countReset, createTask, debug, dir, dirxml, error, group, groupCollapsed, groupEnd, info,
  log, profile, profileEnd, table, time, timeEnd, timeLog, timeStamp, trace, warn, _ignoreErrors, _stderr,
  _stderrErrorHandler, _stdout, _stdoutErrorHandler, _times,
} = consoleObject;

export default consoleObject;
