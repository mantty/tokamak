import { isNodeBuiltin, nodeBuiltinNames } from "tokamak:host";
import process from "../globals/process.mjs";

const builtinModules = nodeBuiltinNames();

export function isBuiltin(value) {
  return typeof value === "string" && isNodeBuiltin(value);
}

export function createRequire() {
  return function require(name) {
    const value = process.getBuiltinModule(name);
    if (value === undefined) throw new Error(`No such module "${name}".`);
    return value;
  };
}

function notImplemented(name) {
  const error = new Error(`The module.${name} method is not implemented`);
  error.code = "ERR_METHOD_NOT_IMPLEMENTED";
  throw error;
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
export function wrap(source) { return ["(function (exports, require, module, __filename, __dirname) {", String(source), "\n});"]; }

export function Module(id, parent) {
  if (!(this instanceof Module)) return new Module(id, parent);
  this.id = id === undefined ? "." : String(id);
  this.filename = this.id;
  this.path = ".";
  this.paths = [];
  this.parent = parent ?? null;
  this.children = [];
  this.loaded = false;
  this.exports = {};
}

Module.prototype.load = function () { return notImplemented("load"); };
Module.prototype.require = createRequire();
Module.prototype.isPreloading = false;
Object.assign(Module, { builtinModules, isBuiltin, createRequire, Module, _cache: {}, _debug: false });
Module._findPath = () => notImplemented("_findPath");
Module._initPaths = () => notImplemented("_initPaths");
Module._load = () => notImplemented("_load");
Module._nodeModulePaths = () => [];
Module._preloadModules = [];
Module._resolveFilename = () => notImplemented("_resolveFilename");
Module._resolveLookupPaths = () => notImplemented("_resolveLookupPaths");
Object.assign(Module, {
  _pathCache: {},
  _extensions: {},
  globalPaths: [],
  constants: { compileCacheStatus: { FAILED: 0, ENABLED: 1, ALREADY_ENABLED: 2, DISABLED: 3 } },
  SourceMap, stripTypeScriptTypes, register, registerHooks, runMain, findPackageJSON, getCompileCacheDir, findSourceMap,
  getSourceMapsSupport, setSourceMapsSupport, enableCompileCache, flushCompileCache, syncBuiltinESMExports, wrap,
});

export { builtinModules };
export default Module;
