import { installWebGlobals } from "../globals/web.mjs";
import { installWebSocketGlobals } from "../network/websocket.mjs";

installWebGlobals();
installWebSocketGlobals();
globalThis.global = globalThis;
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
