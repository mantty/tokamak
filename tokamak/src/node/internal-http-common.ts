export const CRLF = "\r\n";
export const chunkExpression = /^(?:[0-9a-f]+)\r?\n/i;
export const continueExpression = /^100\s/;
export const kIncomingMessage = Symbol("kIncomingMessage");
export const methods = [];
export const parsers = {};
export function _checkInvalidHeaderChar(value) { return /[\r\n]/.test(String(value)); }
export function _checkIsHttpToken(value) { return /^[!#$%&'*+.^_`|~0-9A-Za-z-]+$/.test(String(value)); }

export default { CRLF, _checkInvalidHeaderChar, _checkIsHttpToken, chunkExpression, continueExpression, kIncomingMessage, methods, parsers };
