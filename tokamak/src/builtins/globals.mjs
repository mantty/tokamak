import { installConsoleGlobal } from "../globals/console.mjs";
import { installProcessGlobals } from "../globals/process.mjs";
import { installWebGlobals } from "../globals/web.mjs";
import { installWebSocketGlobals } from "../network/websocket.mjs";
import { Buffer } from "../node/buffer.mjs";

installWebGlobals();
installWebSocketGlobals();
installProcessGlobals();
installConsoleGlobal();
globalThis.global = globalThis;
globalThis.Buffer = Buffer;
delete globalThis.InternalError;

// Array/TypedArray toLocaleString forward locales and options to each element,
// which the engine's own implementation does not.
function localeElements(locales, options) {
  let result = "";
  for (let index = 0; index < this.length; index += 1) {
    if (index > 0) result += ",";
    const element = this[index];
    if (element !== undefined && element !== null) result += element.toLocaleString(locales, options);
  }
  return result;
}
for (const prototype of [Array.prototype, Object.getPrototypeOf(Uint8Array.prototype)]) {
  Object.defineProperty(prototype, "toLocaleString", { value: localeElements, writable: true, configurable: true });
}
