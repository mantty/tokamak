import { nodeBuiltinNames } from "tokamak:host";
import { unsupportedFunction } from "./unsupported.mjs";

export class REPLServer {}
export const REPL_MODE_SLOPPY = Symbol("REPL_MODE_SLOPPY");
export const REPL_MODE_STRICT = Symbol("REPL_MODE_STRICT");
export class Recoverable {}
export const builtinModules = nodeBuiltinNames().filter(name => !name.startsWith("_"));
export const _builtinLibs = builtinModules;
export const start = unsupportedFunction("repl.start");
export const writer = value => String(value);

export default { REPLServer, REPL_MODE_SLOPPY, REPL_MODE_STRICT, Recoverable, _builtinLibs, builtinModules, start, writer };
