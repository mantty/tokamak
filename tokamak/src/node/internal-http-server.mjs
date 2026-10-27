import { STATUS_CODES, Server, ServerResponse, _connectionListener } from "./http.mjs";

export const httpServerPreClose = Symbol("httpServerPreClose");
export const kConnectionsCheckingInterval = Symbol("kConnectionsCheckingInterval");
export const kServerResponse = Symbol("kServerResponse");
export function setupConnectionsTracking() {}
export function storeHTTPOptions() {}

export { STATUS_CODES, Server, ServerResponse, _connectionListener };
export default {
  STATUS_CODES, Server, ServerResponse, _connectionListener, httpServerPreClose, kConnectionsCheckingInterval, kServerResponse,
  setupConnectionsTracking, storeHTTPOptions,
};
