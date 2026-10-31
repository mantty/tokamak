import { builtinNamespace, isNodeBuiltin, processChdir, processCwd } from "tokamak:host";
import EventEmitter from "../events/events.ts";
import { setWarningProcess } from "../events/process-warnings.ts";
import { usvString } from "./conversions.ts";
import { isFunction, objectOf } from "./objects.ts";

const builtinModules = new Map<string, unknown>();
const processImpl = { getCwd: () => processCwd() };

const process = Object.assign(new EventEmitter(), {
  env: Object.fromEntries(Object.entries(globalThis.__tokamak_env ?? {}).map(([key, value]) => [key, typeof value === "string" ? value : JSON.stringify(value)])),
  nextTick: (callback: unknown, ...args: unknown[]) => queueMicrotask(() => {
    if (!isFunction(callback)) throw new TypeError("The \"callback\" argument must be of type function");
    callback(...args);
  }),
  // A Node module's default export, or another builtin's namespace, kept once found.
  getBuiltinModule: (specifier: unknown): unknown => {
    const name = usvString(specifier);
    if (builtinModules.has(name)) return builtinModules.get(name);
    const namespace = builtinNamespace(name);
    const module = namespace && isNodeBuiltin(name) ? namespace.default : namespace;
    if (module !== undefined) builtinModules.set(name, module);
    return module;
  },
  argv: ["workerd"],
  argv0: "workerd",
  execArgv: [],
  execPath: "",
  pid: 1,
  ppid: 0,
  title: "workerd",
  platform: "linux",
  arch: "x64",
  version: "v22.19.0",
  versions: { node: "22.19.0", v8: "", modules: "", zlib: "", openssl: "", icu: "", tz: "" },
  release: { name: "node", lts: true, sourceUrl: "", headersUrl: "" },
  exitCode: 0,
  umask: () => 0o22,
  uptime: () => 0,
  memoryUsage: () => ({ rss: 0, heapTotal: 0, heapUsed: 0, external: 0, arrayBuffers: 0 }),
  resourceUsage: () => ({}),
  cpuUsage: () => ({ user: 0, system: 0 }),
  stdout: { write() { return true; } },
  stderr: { write() { return true; } },
  stdin: { isTTY: false },
  exit: (code?: unknown) => { globalThis.process.exitCode = Number(code ?? 0); },
  abort: () => { throw new Error("process.abort is not available in the Tokamak runtime"); },
  cwd: processImpl.getCwd.bind(processImpl),
  hrtime: (start?: unknown) => {
    const now = Date.now();
    const value = [Math.floor(now / 1000), (now % 1000) * 1e6];
    if (!start) return value;
    const seconds = value[0] - Number(objectOf(start)[0]);
    const nanos = value[1] - Number(objectOf(start)[1]);
    return nanos < 0 ? [seconds - 1, nanos + 1e9] : [seconds, nanos];
  },
}, {
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
  _rawDebug: (...args: unknown[]) => console.error(...args),
  _send: () => false,
  _startProfilerIdleNotifier: () => {},
  _stopProfilerIdleNotifier: () => {},
  _tickCallback: () => {},
  allowedNodeEnvironmentFlags: new Set<string>(),
  assert: (value: unknown) => { if (!value) throw new Error("Assertion failed"); },
  availableMemory: () => 0,
  binding: () => { throw new Error("process.binding is not available in the Tokamak runtime"); },
  channel: () => undefined,
  chdir: (directory: unknown) => processChdir(directory),
  config: {},
  connected: false,
  constrainedMemory: () => 0,
  debugPort: 0,
  dlopen: () => { throw new Error("process.dlopen is not available in the Tokamak runtime"); },
  domain: undefined,
  emitWarning: (warning: unknown) => console.warn(warning),
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
  reallyExit: (code?: unknown) => { globalThis.process.exitCode = Number(code ?? 0); },
  ref: (value: unknown) => value,
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
  unref: (value: unknown) => value,
});
setWarningProcess(process);
export default process;
