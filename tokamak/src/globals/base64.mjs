import { string } from "./conversions.mjs";
import { DOMException } from "./dom-exception.mjs";

export function atob(value) {
  if (arguments.length === 0) throw new TypeError("Failed to execute 'atob' on 'ServiceWorkerGlobalScope': parameter 1 is not of type 'string'.");
  const input = string(value);
  let bytes;
  try { bytes = Uint8Array.fromBase64(input); }
  catch { throw new DOMException("atob() called with invalid base64-encoded data. (Only whitespace, '+', '/', alphanumeric ASCII, and up to two terminal '=' signs when the input data length is divisible by 4 are allowed.)", "InvalidCharacterError"); }
  let output = "";
  for (const byte of bytes) output += String.fromCharCode(byte);
  return output;
}

export function btoa(value) {
  if (arguments.length === 0) throw new TypeError("Failed to execute 'btoa' on 'ServiceWorkerGlobalScope': parameter 1 is not of type 'String'.");
  const input = string(value);
  if (/[^\0-\xff]/.test(input)) throw new DOMException("btoa() can only operate on characters in the Latin1 (ISO/IEC 8859-1) range.", "InvalidCharacterError");
  return Uint8Array.from(input, character => character.charCodeAt(0)).toBase64();
}
