import EventEmitter from "../events/events.mjs";
import { Socket } from "./net.mjs";
import { unsupportedFunction } from "./unsupported.mjs";

export const CLIENT_RENEG_LIMIT = 3;
export const CLIENT_RENEG_WINDOW = 600;
export const DEFAULT_CIPHERS = "";
export const DEFAULT_ECDH_CURVE = "auto";
export const DEFAULT_MAX_VERSION = "TLSv1.3";
export const DEFAULT_MIN_VERSION = "TLSv1.2";

export class SecureContext {
  constructor(options = {}) { this.options = { ...options }; }
}

export class Server extends EventEmitter {}

export class TLSSocket extends Socket {
  constructor(options = {}) {
    super({ ...options, secureTransport: "on" });
    Object.defineProperty(this, "encrypted", { configurable: true, enumerable: true, value: true });
  }

  _destroySSL() {}
  _emitTLSError(error) { this.emit("error", error); }
  _finishInit() {}
  _handleTimeout() { this.emit("timeout"); }
  _init() {}
  _releaseControl() {}
  _start() {}
  _tlsError(error) { this.emit("error", error); }
  _wrapHandle() {}
  disableRenegotiation() {}
  enableTrace() {}
  exportKeyingMaterial() { return unsupportedFunction("tls.TLSSocket.exportKeyingMaterial")(); }
  getCertificate() { return undefined; }
  getCipher() { return undefined; }
  getEphemeralKeyInfo() { return undefined; }
  getFinished() { return undefined; }
  getPeerCertificate() { return {}; }
  getPeerFinished() { return undefined; }
  getPeerX509Certificate() { return undefined; }
  getProtocol() { return undefined; }
  getSession() { return undefined; }
  getSharedSigalgs() { return []; }
  getTLSTicket() { return undefined; }
  getX509Certificate() { return undefined; }
  isSessionReused() { return false; }
  renegotiate(_options, callback) { callback?.(new Error("TLS renegotiation is not supported")); return false; }
  setKeyCert() { return this; }
  setMaxSendFragment(size) { return Number.isInteger(size) && size > 0; }
  setServername(name) { this.__servername = String(name); return this; }
  setSession() { return this; }
}

export function checkServerIdentity() { return undefined; }

export function connect(...args) {
  const callback = typeof args.at(-1) === "function" ? args.at(-1) : undefined;
  const first = args[0];
  const options = first !== null && typeof first === "object"
    ? { ...first, secureTransport: "on" }
    : { port: first, host: args[1], secureTransport: "on" };
  const socket = new TLSSocket(options);
  socket.connect(options, callback);
  return socket;
}

export function convertALPNProtocols(value) { return value; }
export function createSecureContext(options) { return new SecureContext(options); }
export const createSecurePair = unsupportedFunction("tls.createSecurePair");
export const createServer = unsupportedFunction("tls.createServer");
export function getCiphers() { return []; }
export const rootCertificates = [];
export function translatePeerCertificate(certificate) { return certificate; }

export default {
  CLIENT_RENEG_LIMIT, CLIENT_RENEG_WINDOW, DEFAULT_CIPHERS, DEFAULT_ECDH_CURVE, DEFAULT_MAX_VERSION, DEFAULT_MIN_VERSION,
  SecureContext, Server, TLSSocket, checkServerIdentity, connect, convertALPNProtocols, createSecureContext, createSecurePair,
  createServer, getCiphers, rootCertificates,
};
