import { unsupportedFunction } from "./unsupported.mjs";

export class Script {
  constructor() { unsupportedFunction("vm.Script")(); }
}
export const compileFunction = unsupportedFunction("vm.compileFunction");
export const constants = {};
export const createContext = unsupportedFunction("vm.createContext");
export const createScript = unsupportedFunction("vm.createScript");
export const isContext = () => false;
export const measureMemory = unsupportedFunction("vm.measureMemory");
export const runInContext = unsupportedFunction("vm.runInContext");
export const runInNewContext = unsupportedFunction("vm.runInNewContext");
export const runInThisContext = unsupportedFunction("vm.runInThisContext");

export default { Script, compileFunction, constants, createContext, createScript, isContext, measureMemory, runInContext, runInNewContext, runInThisContext };
