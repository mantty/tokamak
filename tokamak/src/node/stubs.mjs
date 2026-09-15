import { MessageChannel, MessagePort } from "../events/web.mjs";

function unsupported(name) { return (...args) => { throw new Error(`${name} is not available in the Tokamak runtime`); }; }
export const spawn = unsupported("child_process.spawn");
export const exec = unsupported("child_process.exec");
export const execFile = unsupported("child_process.execFile");
export const fork = unsupported("child_process.fork");
export const spawnSync = unsupported("child_process.spawnSync");
export const execSync = unsupported("child_process.execSync");
export const execFileSync = unsupported("child_process.execFileSync");
export const createSocket = unsupported("dgram.createSocket");
export const createDomain = unsupported("domain.create");
export const create = createDomain;
export const open = unsupported("inspector.open");
export const close = unsupported("inspector.close");
export const runInNewContext = unsupported("vm.runInNewContext");
export const runInThisContext = unsupported("vm.runInThisContext");
export const compileFunction = unsupported("vm.compileFunction");
export const start = unsupported("repl.start");
export const getHeapStatistics = unsupported("v8.getHeapStatistics");
export const WASI = class { constructor() { throw new Error("wasi is not available in the Tokamak runtime"); } };
export const Worker = class { constructor() { throw new Error("worker_threads.Worker is not available in the Tokamak runtime"); } };
export const BroadcastChannel = class {};
export { MessageChannel, MessagePort };
export const isMainThread = true;
export const threadId = 0;
export const workerData = undefined;
export const parentPort = null;
export const SHARE_ENV = Symbol("SHARE_ENV");
export const mock = {};
export class MockTracker {}
export class MockFunctionContext {}
export default {
  spawn, exec, execFile, fork, spawnSync, execSync, createSocket, createDomain, create,
  open, close, runInNewContext, runInThisContext, compileFunction, start, getHeapStatistics, WASI,
  Worker, BroadcastChannel, MessageChannel, MessagePort, isMainThread, threadId, workerData, parentPort,
  SHARE_ENV, mock, MockTracker, MockFunctionContext,
};
