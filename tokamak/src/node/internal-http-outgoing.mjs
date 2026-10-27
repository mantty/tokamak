import { OutgoingMessage, validateHeaderName, validateHeaderValue } from "./http.mjs";

export const kHighWaterMark = Symbol("kHighWaterMark");
export const kUniqueHeaders = Symbol("kUniqueHeaders");
export function parseUniqueHeadersOption(value) { return value; }

export { OutgoingMessage, validateHeaderName, validateHeaderValue };
export default { OutgoingMessage, kHighWaterMark, kUniqueHeaders, parseUniqueHeadersOption, validateHeaderName, validateHeaderValue };
