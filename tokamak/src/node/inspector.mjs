import { unsupportedFunction } from "./unsupported.mjs";

export class Network {}
export class Session {}
export const close = unsupportedFunction("inspector.close");
export const console = {};
export const open = unsupportedFunction("inspector.open");
export const url = () => undefined;
export const waitForDebugger = unsupportedFunction("inspector.waitForDebugger");
export default { Network, Session, close, console, open, url, waitForDebugger };
