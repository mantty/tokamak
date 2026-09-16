import { unsupportedFunction } from "./unsupported.mjs";

export class Socket {}
export const createSocket = unsupportedFunction("dgram.createSocket");
export default { Socket, createSocket };
