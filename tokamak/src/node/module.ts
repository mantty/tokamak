import { isNodeBuiltin, nodeBuiltinNames } from "tokamak:host";
import process from "../globals/process.ts";
import { codedError } from "./errors.ts";

const builtinModules = nodeBuiltinNames();

interface Module {
  id: string;
  filename: string;
  path: string;
  paths: string[];
  parent: unknown;
  children: Module[];
  loaded: boolean;
  exports: unknown;
  load(): never;
  require(name: unknown): unknown;
  isPreloading: boolean;
}

interface ModuleConstructor {
  new (id?: unknown, parent?: unknown): Module;
  (id?: unknown, parent?: unknown): Module;
  readonly prototype: Module;
}

export function isBuiltin(value: unknown): boolean {
  return typeof value === "string" && isNodeBuiltin(value.toWellFormed());
}

export function createRequire(): (name: unknown) => unknown {
  return function require(name: unknown) {
    const value = process.getBuiltinModule(name);
    if (value === undefined) throw new Error(`No such module "${name}".`);
    return value;
  };
}

function notImplemented(name: string): never {
  throw codedError(Error, `The module.${name} method is not implemented`, "ERR_METHOD_NOT_IMPLEMENTED");
}

export function stripTypeScriptTypes() { return notImplemented("stripTypeScriptTypes"); }
export function register() { return notImplemented("register"); }
export function registerHooks() { return notImplemented("registerHooks"); }
export function runMain() { return notImplemented("runMain"); }
export function findSourceMap() { return undefined; }
export function getSourceMapsSupport() { return { enabled: false, nodeModules: false }; }
export function setSourceMapsSupport() { return notImplemented("setSourceMapsSupport"); }
export function enableCompileCache() { return notImplemented("enableCompileCache"); }
export function flushCompileCache() { return notImplemented("flushCompileCache"); }
export class SourceMap {}
export function findPackageJSON() { return notImplemented("findPackageJSON"); }
export function getCompileCacheDir() { return undefined; }
export function syncBuiltinESMExports() {}
export function wrap(source: unknown): string[] { return ["(function (exports, require, module, __filename, __dirname) {", String(source), "\n});"]; }

// A function rather than a class, as in Node, so that it constructs a module
// when called without `new`. TypeScript gives a function expression no
// construct signature, so its type is stated.
export const Module = function (this: Module | undefined, id?: unknown, parent?: unknown) {
  if (!(this instanceof Module)) return new Module(id, parent);
  this.id = id === undefined ? "." : String(id);
  this.filename = this.id;
  this.path = ".";
  this.paths = [];
  this.parent = parent ?? null;
  this.children = [];
  this.loaded = false;
  this.exports = {};
} as ModuleConstructor;

Module.prototype.load = function () { return notImplemented("load"); };
Module.prototype.require = createRequire();
Module.prototype.isPreloading = false;
Object.assign(Module, {
  builtinModules, isBuiltin, createRequire, Module, _cache: {}, _debug: false,
  _findPath: () => notImplemented("_findPath"),
  _initPaths: () => notImplemented("_initPaths"),
  _load: () => notImplemented("_load"),
  _nodeModulePaths: () => [],
  _preloadModules: [],
  _resolveFilename: () => notImplemented("_resolveFilename"),
  _resolveLookupPaths: () => notImplemented("_resolveLookupPaths"),
  _pathCache: {},
  _extensions: {},
  globalPaths: [],
  constants: { compileCacheStatus: { FAILED: 0, ENABLED: 1, ALREADY_ENABLED: 2, DISABLED: 3 } },
  SourceMap, stripTypeScriptTypes, register, registerHooks, runMain, findPackageJSON, getCompileCacheDir, findSourceMap,
  getSourceMapsSupport, setSourceMapsSupport, enableCompileCache, flushCompileCache, syncBuiltinESMExports, wrap,
});

export { builtinModules };
export default Module;
