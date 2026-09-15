import { unsupportedFunction } from "./unsupported.mjs";

export class REPLServer {}
export const REPL_MODE_SLOPPY = Symbol("REPL_MODE_SLOPPY");
export const REPL_MODE_STRICT = Symbol("REPL_MODE_STRICT");
export class Recoverable {}
export const _builtinLibs = [];
export const builtinModules = [];
export const start = unsupportedFunction("repl.start");
export const writer = value => String(value);

export default { REPLServer, REPL_MODE_SLOPPY, REPL_MODE_STRICT, Recoverable, _builtinLibs, builtinModules, start, writer };
