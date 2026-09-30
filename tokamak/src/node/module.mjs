const builtinModules = [
  "assert", "assert/strict", "async_hooks", "buffer", "child_process", "cluster", "console", "constants",
  "crypto", "dgram", "diagnostics_channel", "dns", "dns/promises", "domain", "events", "fs", "fs/promises",
  "http", "http2", "https", "inspector", "inspector/promises", "module", "net", "os", "path", "path/posix",
  "path/win32", "perf_hooks", "process", "punycode", "querystring", "readline", "readline/promises", "repl",
  "stream", "stream/consumers", "stream/promises", "stream/web", "string_decoder", "sys", "timers",
  "timers/promises", "tls", "trace_events", "tty", "url", "util", "util/types", "v8", "vm", "wasi",
  "worker_threads", "zlib", "_http_agent", "_http_client", "_http_common", "_http_incoming", "_http_outgoing",
  "_http_server", "_stream_duplex", "_stream_passthrough", "_stream_readable", "_stream_transform", "_stream_wrap",
  "_stream_writable", "_tls_common", "_tls_wrap", "sqlite", "test",
];

function nameOf(value) {
  if (typeof value !== "string") throw new TypeError("The module name must be a string");
  return value.startsWith("node:") ? value.slice(5) : value;
}

function moduleNotFound(key) {
  const error = new Error(`Cannot find module '${key}'`);
  error.code = "MODULE_NOT_FOUND";
  return error;
}

export function isBuiltin(value) {
  return typeof value === "string" && (value.startsWith("node:") || builtinModules.includes(nameOf(value)));
}

export function createRequire() {
  const require = name => {
    const key = nameOf(name);
    const value = globalThis.process?.getBuiltinModule?.(key);
    if (value !== undefined) return value;
    throw moduleNotFound(key);
  };
  require.resolve = name => {
    const key = nameOf(name);
    if (!isBuiltin(key)) throw moduleNotFound(key);
    return `node:${key}`;
  };
  require.cache = {};
  require.extensions = {};
  return require;
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
