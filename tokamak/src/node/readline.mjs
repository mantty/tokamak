import { unsupportedFunction } from "./unsupported.mjs";

export class Interface {}
export const clearLine = unsupportedFunction("readline.clearLine");
export const clearScreenDown = unsupportedFunction("readline.clearScreenDown");
export const createInterface = unsupportedFunction("readline.createInterface");
export const cursorTo = unsupportedFunction("readline.cursorTo");
export const emitKeypressEvents = unsupportedFunction("readline.emitKeypressEvents");
export const moveCursor = unsupportedFunction("readline.moveCursor");
export const promises = {};

export default { Interface, clearLine, clearScreenDown, createInterface, cursorTo, emitKeypressEvents, moveCursor, promises };
