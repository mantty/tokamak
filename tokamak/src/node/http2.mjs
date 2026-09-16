import { unsupportedFunction } from "./unsupported.mjs";

export class Http2ServerRequest {}
export class Http2ServerResponse {}
export const connect = unsupportedFunction("http2.connect");
export const constants = {};
export const createSecureServer = unsupportedFunction("http2.createSecureServer");
export const createServer = unsupportedFunction("http2.createServer");
export const getDefaultSettings = () => ({});
export const getPackedSettings = unsupportedFunction("http2.getPackedSettings");
export const getUnpackedSettings = unsupportedFunction("http2.getUnpackedSettings");
export const performServerHandshake = unsupportedFunction("http2.performServerHandshake");
export const sensitiveHeaders = Symbol("http2.sensitiveHeaders");

export default {
  Http2ServerRequest, Http2ServerResponse, connect, constants, createSecureServer, createServer,
  getDefaultSettings, getPackedSettings, getUnpackedSettings, performServerHandshake, sensitiveHeaders,
};
