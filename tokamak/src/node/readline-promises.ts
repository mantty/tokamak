import { Interface } from "./readline.ts";
import { unsupportedFunction } from "./unsupported.ts";

export { Interface };
export class Readline {}
export const createInterface = unsupportedFunction("readline/promises.createInterface");
export default { Interface, Readline, createInterface };
