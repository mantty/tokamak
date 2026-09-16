import EventEmitter from "../events/events.mjs";
import { unsupported, unsupportedFunction } from "./unsupported.mjs";

export class ChildProcess extends EventEmitter {
  kill() { return unsupported("child_process.ChildProcess.kill()"); }
}

export const _forkChild = unsupportedFunction("child_process._forkChild");
export const exec = unsupportedFunction("child_process.exec");
export const execFile = unsupportedFunction("child_process.execFile");
export const execFileSync = unsupportedFunction("child_process.execFileSync");
export const execSync = unsupportedFunction("child_process.execSync");
export const fork = unsupportedFunction("child_process.fork");
export const spawn = unsupportedFunction("child_process.spawn");
export const spawnSync = unsupportedFunction("child_process.spawnSync");

export default { ChildProcess, _forkChild, exec, execFile, execFileSync, execSync, fork, spawn, spawnSync };
