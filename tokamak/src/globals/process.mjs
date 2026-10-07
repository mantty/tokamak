import { builtinNamespace, isNodeBuiltin } from "tokamak:host";
import EventEmitter from "../events/events.mjs";
import { string } from "./conversions.mjs";

const process = new EventEmitter();
export default process;

const builtinModules = new Map();

// The builtin module `name` names: a Node module's default export, or another's namespace.
function builtinModule(name) {
  const namespace = builtinNamespace(name);
  return namespace && isNodeBuiltin(name) ? namespace.default : namespace;
}

export function installProcessGlobals() {
  globalThis.process = process;
  process.env = Object.fromEntries(Object.entries(globalThis.__tokamak_env ?? {}).map(([key, value]) => [key, typeof value === "string" ? value : JSON.stringify(value)]));
  process.nextTick = (callback, ...args) => queueMicrotask(() => callback(...args));
  process.getBuiltinModule = specifier => {
    const name = string(specifier);
    if (!builtinModules.has(name)) builtinModules.set(name, builtinModule(name));
    return builtinModules.get(name);
  };
  process.argv = ["workerd"];
  process.argv0 = "workerd";
  process.execArgv = [];
  process.execPath = "";
  process.pid = 1;
  process.ppid = 0;
  process.title = "workerd";
  process.platform = "linux";
  process.arch = "x64";
  process.version = "v22.19.0";
  process.versions = { node: "22.19.0", v8: "", modules: "", zlib: "", openssl: "", icu: "", tz: "" };
  process.release = { name: "node", lts: true, sourceUrl: "", headersUrl: "" };
  process.exitCode = 0;
  process.umask = () => 0o22;
  process.uptime = () => 0;
  process.memoryUsage = () => ({ rss: 0, heapTotal: 0, heapUsed: 0, external: 0, arrayBuffers: 0 });
  process.resourceUsage = () => ({});
  process.cpuUsage = () => ({ user: 0, system: 0 });
  process.stdout = { write() { return true; } };
  process.stderr = { write() { return true; } };
  process.stdin = { isTTY: false };
  process.exit = code => { globalThis.process.exitCode = Number(code ?? 0); };
  process.abort = () => { throw new Error("process.abort is not available in the Tokamak runtime"); };
  process.cwd = () => "/bundle";
  process.hrtime = (start) => {
    const now = Date.now();
    const value = [Math.floor(now / 1000), (now % 1000) * 1e6];
    if (!start) return value;
    const seconds = value[0] - start[0];
    const nanos = value[1] - start[1];
    return nanos < 0 ? [seconds - 1, nanos + 1e9] : [seconds, nanos];
  };
  Object.assign(process, {
    _channel: undefined,
    _debugEnd: () => {},
    _debugProcess: () => {},
    _disconnect: () => {},
    _events: Object.create(null),
    _eventsCount: 0,
    _exiting: false,
    _fatalException: () => false,
    _getActiveHandles: () => [],
    _getActiveRequests: () => [],
    _handleQueue: [],
    _kill: () => {},
    _linkedBinding: () => { throw new Error("process._linkedBinding is not available in the Tokamak runtime"); },
    _maxListeners: undefined,
    _pendingMessage: undefined,
    _preload_modules: [],
    _rawDebug: (...args) => console.error(...args),
    _send: () => false,
    _startProfilerIdleNotifier: () => {},
    _stopProfilerIdleNotifier: () => {},
    _tickCallback: () => {},
    allowedNodeEnvironmentFlags: new Set(),
    assert: value => { if (!value) throw new Error("Assertion failed"); },
    availableMemory: () => 0,
    binding: () => { throw new Error("process.binding is not available in the Tokamak runtime"); },
    channel: () => undefined,
    chdir: () => { throw new Error("process.chdir is not available in the Tokamak runtime"); },
    config: {},
    connected: false,
    constrainedMemory: () => 0,
    debugPort: 0,
    dlopen: () => { throw new Error("process.dlopen is not available in the Tokamak runtime"); },
    domain: undefined,
    emitWarning: warning => console.warn(warning),
    execve: () => { throw new Error("process.execve is not available in the Tokamak runtime"); },
    features: {},
    finalization: {},
    getActiveResourcesInfo: () => [],
    getSourceMapsSupport: () => ({ enabled: false, nodeModules: false }),
    getegid: () => 0,
    geteuid: () => 0,
    getgid: () => 0,
    getgroups: () => [],
    getuid: () => 0,
    hasUncaughtExceptionCaptureCallback: () => false,
    initgroups: () => {},
    kill: () => false,
    loadEnvFile: () => {},
    moduleLoadList: [],
    noDeprecation: false,
    openStdin: () => globalThis.process.stdin,
    permission: {},
    reallyExit: code => { globalThis.process.exitCode = Number(code ?? 0); },
    ref: value => value,
    report: {},
    send: () => false,
    setSourceMapsEnabled: () => {},
    setUncaughtExceptionCaptureCallback: () => {},
    setegid: () => {},
    seteuid: () => {},
    setgid: () => {},
    setgroups: () => {},
    setuid: () => {},
    sourceMapsEnabled: false,
    threadCpuUsage: () => ({ user: 0, system: 0 }),
    throwDeprecation: false,
    traceDeprecation: false,
    unref: value => value,
  });
}
