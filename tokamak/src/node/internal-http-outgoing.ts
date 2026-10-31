import { OutgoingMessage, validateHeaderName, validateHeaderValue } from "./http.ts";

export const kHighWaterMark = Symbol("kHighWaterMark");
export const kUniqueHeaders = Symbol("kUniqueHeaders");
export function parseUniqueHeadersOption<T>(value: T): T { return value; }

export { OutgoingMessage, validateHeaderName, validateHeaderValue };
export default { OutgoingMessage, kHighWaterMark, kUniqueHeaders, parseUniqueHeadersOption, validateHeaderName, validateHeaderValue };
