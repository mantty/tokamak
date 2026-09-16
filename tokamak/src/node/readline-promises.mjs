import { Interface } from "./readline.mjs";
import { unsupportedFunction } from "./unsupported.mjs";

export { Interface };
export class Readline {}
export const createInterface = unsupportedFunction("readline/promises.createInterface");
export default { Interface, Readline, createInterface };
