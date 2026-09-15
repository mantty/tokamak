import EventEmitter from "../events/events.mjs";

const process = new EventEmitter();
export default process;

export function installProcessGlobals(builtinModules) {
  globalThis.process = process;
  globalThis.process.env = Object.fromEntries(Object.entries(globalThis.__tokamak_env ?? {}).map(([key, value]) => [key, typeof value === "string" ? value : JSON.stringify(value)]));
  globalThis.process.nextTick = (callback, ...args) => queueMicrotask(() => callback(...args));
  globalThis.process.getBuiltinModule = (name) => {
    if (typeof name !== "string") throw new TypeError("Module name must be a string");
    const key = name.startsWith("node:") ? name.slice(5) : name;
    return Object.hasOwn(builtinModules, key) ? builtinModules[key] : undefined;
  };
  globalThis.process.argv = ["workerd"];
  globalThis.process.argv0 = "workerd";
  globalThis.process.execArgv = [];
  globalThis.process.execPath = "";
  globalThis.process.pid = 1;
  globalThis.process.ppid = 0;
  globalThis.process.title = "workerd";
  globalThis.process.platform = "linux";
  globalThis.process.arch = "x64";
  globalThis.process.version = "v22.19.0";
  globalThis.process.versions = { node: "22.19.0", v8: "", modules: "", zlib: "", openssl: "", icu: "", tz: "" };
  globalThis.process.release = { name: "node", lts: true, sourceUrl: "", headersUrl: "" };
  globalThis.process.exitCode = 0;
  globalThis.process.umask = () => 0o22;
  globalThis.process.uptime = () => 0;
  globalThis.process.memoryUsage = () => ({ rss: 0, heapTotal: 0, heapUsed: 0, external: 0, arrayBuffers: 0 });
  globalThis.process.resourceUsage = () => ({});
  globalThis.process.cpuUsage = () => ({ user: 0, system: 0 });
  globalThis.process.stdout = { write() { return true; } };
  globalThis.process.stderr = { write() { return true; } };
  globalThis.process.stdin = { isTTY: false };
  globalThis.process.exit = code => { globalThis.process.exitCode = Number(code ?? 0); };
  globalThis.process.abort = () => { throw new Error("process.abort is not available in the Tokamak runtime"); };
  globalThis.process.cwd = () => "/bundle";
  globalThis.process.hrtime = (start) => {
    const now = Date.now();
    const value = [Math.floor(now / 1000), (now % 1000) * 1e6];
    if (!start) return value;
    const seconds = value[0] - start[0];
    const nanos = value[1] - start[1];
    return nanos < 0 ? [seconds - 1, nanos + 1e9] : [seconds, nanos];
  };
  Object.assign(globalThis.process, {
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
