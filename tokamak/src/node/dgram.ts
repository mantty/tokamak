import { unsupportedFunction } from "./unsupported.ts";

export class Socket {}
export const createSocket = unsupportedFunction("dgram.createSocket");
export default { Socket, createSocket };
