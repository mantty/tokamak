import { decodeBase64, encodeBase64 } from "tokamak:host";
import { string } from "./conversions.mjs";
import { DOMException } from "./dom-exception.mjs";

export function atob(value) {
  if (arguments.length === 0) throw new TypeError("Failed to execute 'atob' on 'ServiceWorkerGlobalScope': parameter 1 is not of type 'string'.");
  const input = string(value).replace(/[\t\n\f\r ]/g, "");
  const valid = !input.includes("=") || (input.length % 4 === 0 && /^[A-Za-z0-9+/]*={1,2}$/.test(input));
  const bytes = valid ? decodeBase64(input) : null;
  if (!bytes) throw new DOMException("atob() called with invalid base64-encoded data. (Only whitespace, '+', '/', alphanumeric ASCII, and up to two terminal '=' signs when the input data length is divisible by 4 are allowed.)", "InvalidCharacterError");
  let output = "";
  for (const byte of bytes) output += String.fromCharCode(byte);
  return output;
}

export function btoa(value) {
  if (arguments.length === 0) throw new TypeError("Failed to execute 'btoa' on 'ServiceWorkerGlobalScope': parameter 1 is not of type 'String'.");
  const input = string(value);
  const bytes = new Uint8Array(input.length);
  for (let index = 0; index < input.length; index += 1) {
    const code = input.charCodeAt(index);
    if (code > 255) throw new DOMException("btoa() can only operate on characters in the Latin1 (ISO/IEC 8859-1) range.", "InvalidCharacterError");
    bytes[index] = code;
  }
  return encodeBase64(bytes);
}
