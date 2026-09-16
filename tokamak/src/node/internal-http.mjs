import {
  Agent,
  ClientRequest,
  IncomingMessage,
  OutgoingMessage,
  Server,
  ServerResponse,
  STATUS_CODES,
  globalAgent,
} from "./http.mjs";

export { Agent, ClientRequest, IncomingMessage, OutgoingMessage, Server, ServerResponse, STATUS_CODES, globalAgent };
export const CRLF = "\r\n";
export const chunkExpression = /^(?:[0-9a-f]+)\r?\n/i;
export const continueExpression = /^100\s/;
export const kIncomingMessage = Symbol("kIncomingMessage");
export const kHighWaterMark = Symbol("kHighWaterMark");
export const kServerResponse = Symbol("kServerResponse");
export const kUniqueHeaders = Symbol("kUniqueHeaders");
export const methods = [];
export const parsers = {};
export const httpServerPreClose = Symbol("httpServerPreClose");
export const kConnectionsCheckingInterval = Symbol("kConnectionsCheckingInterval");
export function _checkInvalidHeaderChar(value) { return /[\r\n]/.test(String(value)); }
export function _checkIsHttpToken(value) { return /^[!#$%&'*+.^_`|~0-9A-Za-z-]+$/.test(String(value)); }
export function parseUniqueHeadersOption(value) { return value; }
export function setupConnectionsTracking() {}
export function storeHTTPOptions() {}
export function validateHeaderName(name) {
  if (!_checkIsHttpToken(name)) throw new TypeError(`Invalid header name: ${name}`);
  return name;
}
export function validateHeaderValue(name, value) {
  if (_checkInvalidHeaderChar(value)) throw new TypeError(`Invalid value for header ${name}`);
  return value;
}
export function _connectionListener() {}

export default {
  Agent,
  ClientRequest,
  IncomingMessage,
  OutgoingMessage,
  Server,
  ServerResponse,
  STATUS_CODES,
  globalAgent,
  CRLF,
  chunkExpression,
  continueExpression,
  kIncomingMessage,
  kHighWaterMark,
  kServerResponse,
  kUniqueHeaders,
  methods,
  parsers,
  httpServerPreClose,
  kConnectionsCheckingInterval,
  _checkInvalidHeaderChar,
  _checkIsHttpToken,
  parseUniqueHeadersOption,
  setupConnectionsTracking,
  storeHTTPOptions,
  validateHeaderName,
  validateHeaderValue,
  _connectionListener,
};
