export const CRLF = "\r\n";
export const chunkExpression = /^(?:[0-9a-f]+)\r?\n/i;
export const continueExpression = /^100\s/;
export const kIncomingMessage = Symbol("kIncomingMessage");
export const methods: string[] = [];
export const parsers = {};
export function _checkInvalidHeaderChar(value: unknown): boolean { return /[\r\n]/.test(String(value)); }
export function _checkIsHttpToken(value: unknown): boolean { return /^[!#$%&'*+.^_`|~0-9A-Za-z-]+$/.test(String(value)); }

export default { CRLF, _checkInvalidHeaderChar, _checkIsHttpToken, chunkExpression, continueExpression, kIncomingMessage, methods, parsers };
