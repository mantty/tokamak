import { CloseEvent, EventEmitter, MessageEvent } from "../events/events.mjs";
import { WebSocket } from "../network/websocket.mjs";
import { Readable } from "../streams/node.mjs";
import { Buffer } from "./buffer.mjs";

export class Agent {
  constructor(options = {}) { this.options = { ...options }; this.protocol = options.protocol ?? "http:"; }
  destroy() {}
}

function httpServers() {
  if (!globalThis.__tokamak_http_servers) Object.defineProperty(globalThis, "__tokamak_http_servers", { configurable: true, value: new Map() });
  return globalThis.__tokamak_http_servers;
}

export class IncomingMessage extends Readable {
  constructor(source, localPort = 0) {
    super();
    let resolveDone;
    let rejectDone;
    this.__done = new Promise((resolve, reject) => { resolveDone = resolve; rejectDone = reject; });
    if (source?.status !== undefined) {
      this.statusCode = source.status;
      this.statusMessage = source.statusText;
      this.headers = Object.fromEntries(source.headers);
      this.url = source.url;
      this.method = undefined;
    } else {
      const url = new URL(source.url);
      this.method = source.method;
      this.headers = Object.fromEntries(source.headers);
      this.url = url.pathname + (url.search || "");
      this.statusCode = undefined;
      this.statusMessage = undefined;
    }
    this.rawHeaders = Object.entries(this.headers).flatMap(([name, value]) => [name, value]);
    this.rawTrailers = [];
    this.trailers = {};
    this.httpVersion = "1.1";
    this.complete = false;
    this.readable = true;
    this.aborted = false;
    this.cloudflare = { cf: source?.cf ?? {} };
    this.socket = {
      encrypted: source?.url?.startsWith("https:") ?? false,
      remoteFamily: "IPv4",
      remoteAddress: "127.0.0.1",
      remotePort: 32768 + Math.floor(Math.random() * 32768),
      localAddress: this.headers.host ?? "127.0.0.1",
      localPort,
      destroy: () => this.destroy(),
    };
    if (source?.body) void pumpIncomingBody(this, source.body, resolveDone, rejectDone);
    else {
      this.complete = true;
      this.push(null);
      resolveDone();
    }
  }

  destroy(error) {
    this.aborted = true;
    this.__reader?.cancel(error).catch(() => {});
    return super.destroy(error);
  }
}

async function pumpIncomingBody(message, body, resolveDone, rejectDone) {
  try {
    const reader = body.getReader();
    message.__reader = reader;
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      message.push(Buffer.from(value));
    }
    message.complete = true;
    message.push(null);
    resolveDone();
  } catch (error) {
    rejectDone(error);
    message.destroy(error);
  }
}

export class OutgoingMessage extends EventEmitter {
  constructor() {
    super();
    this.headers = new Map();
    this.finished = false;
  }

  setHeader(name, value) { validateHeaderName(name); validateHeaderValue(name, value); this.headers.set(String(name).toLowerCase(), value); return this; }
  getHeader(name) { return this.headers.get(String(name).toLowerCase()); }
  getHeaderNames() { return [...this.headers.keys()]; }
  hasHeader(name) { return this.headers.has(String(name).toLowerCase()); }
  removeHeader(name) { this.headers.delete(String(name).toLowerCase()); }
  flushHeaders() {}
  destroy(error) { if (!this.finished) { this.finished = true; this.__finish?.(); } if (error) this.emit("error", error); return this; }
}

export class ServerResponse extends OutgoingMessage {
  constructor() {
    super();
    this.statusCode = 200;
    this.statusMessage = "OK";
    this.__chunks = [];
    this.__finished = new Promise(resolve => { this.__finish = resolve; });
  }
  writeHead(statusCode, statusMessage, headers) {
    if (typeof statusMessage === "object") { headers = statusMessage; statusMessage = undefined; }
    this.statusCode = Number(statusCode);
    if (statusMessage !== undefined) this.statusMessage = String(statusMessage);
    for (const [name, value] of Object.entries(headers ?? {})) this.setHeader(name, value);
    return this;
  }
  write(chunk, encoding, callback) {
    if (this.finished) throw new Error("write after end");
    if (chunk !== undefined) this.__chunks.push(Buffer.from(chunk, typeof encoding === "string" ? encoding : undefined));
    callback?.();
    return true;
  }
  end(chunk, encoding, callback) {
    if (typeof chunk === "function") callback = chunk;
    else if (chunk !== undefined) this.write(chunk, encoding);
    if (this.finished) return this;
    this.finished = true;
    callback?.();
    this.emit("finish");
    this.__finish();
    return this;
  }

  __response() {
    const headers = new Headers();
    for (const [name, value] of this.headers) {
      if (Array.isArray(value)) for (const item of value) headers.append(name, item);
      else headers.set(name, value);
    }
    return new Response(Buffer.concat(this.__chunks), { status: this.statusCode, statusText: this.statusMessage, headers });
  }
}

export class ClientRequest extends OutgoingMessage {
  constructor(url, options = {}, callback) {
    super();
    this.url = url;
    this.method = String(options.method ?? "GET").toUpperCase();
    this.agent = options.agent ?? globalAgent;
    this.path = new URL(url).pathname + (new URL(url).search || "");
    this._body = [];
    this._ended = false;
    if (typeof callback === "function") this.once("response", callback);
    for (const [name, value] of Object.entries(options.headers ?? {})) this.setHeader(name, value);
  }

  write(chunk, encoding, callback) {
    if (this._ended) throw new Error("write after end");
    this._body.push(Buffer.from(chunk, typeof encoding === "string" ? encoding : undefined));
    callback?.();
    return true;
  }

  end(chunk, encoding, callback) {
    if (this._ended) return this;
    if (typeof chunk === "function") callback = chunk;
    else if (chunk !== undefined) this.write(chunk, encoding);
    this._ended = true;
    const headers = Object.fromEntries(this.headers);
    const body = this._body.length === 0 ? undefined : new Blob([Buffer.concat(this._body)]);
    fetch(this.url, { method: this.method, headers, body }).then(async response => {
      const message = new IncomingMessage(response);
      this.emit("response", message);
      await message.__done;
      callback?.();
      this.emit("finish");
    }, error => {
      this.emit("error", error);
      callback?.(error);
    });
    return this;
  }

  abort() { this.destroy(); }
  destroy(error) { this.__destroyed = true; if (error) this.emit("error", error); this.emit("close"); return this; }
}

export class Server extends EventEmitter {
  constructor(options, requestListener) {
    super();
    if (typeof options === "function") requestListener = options;
    this.options = typeof options === "object" && options !== null ? { ...options } : {};
    this.listening = false;
    if (typeof requestListener === "function") this.on("request", requestListener);
  }

  listen(port, callback) {
    if (typeof port === "function") callback = port;
    else if (port && typeof port === "object") port = port.port;
    const requestedPort = port === undefined ? 0 : Number(port);
    if (!Number.isInteger(requestedPort) || requestedPort < 0 || requestedPort > 65535) throw new RangeError("Invalid port");
    this.__port = requestedPort === 0 ? 32768 + Math.floor(Math.random() * 32768) : requestedPort;
    httpServers().set(this.__port, this);
    this.listening = true;
    callback?.();
    queueMicrotask(() => this.emit("listening"));
    return this;
  }

  address() {
    return this.listening ? { port: this.__port, family: "IPv4", address: "127.0.0.1" } : null;
  }

  close(callback) {
    if (this.listening) httpServers().delete(this.__port);
    this.listening = false;
    callback?.();
    queueMicrotask(() => this.emit("close"));
    return this;
  }

  async __handle(request) {
    const incoming = new IncomingMessage(request, this.__port);
    const response = new ServerResponse();
    try {
      this.emit("request", incoming, response);
    } catch (error) {
      response.destroy(error);
      throw error;
    }
    await response.__finished;
    return response.__response();
  }
}

function requestUrl(input, options) {
  if (input instanceof URL) return input.toString();
  if (typeof input === "string") return input;
  const value = input ?? {};
  const protocol = value.protocol ?? "http:";
  const host = value.hostname ?? value.host ?? "localhost";
  const port = value.port === undefined ? "" : `:${value.port}`;
  const path = value.path ?? "/";
  return `${protocol}//${host}${port}${path}`;
}

function requestOptions(input, options) {
  if (typeof input === "object" && !(input instanceof URL)) return { ...input };
  if (typeof options === "function") return {};
  return { ...(options ?? {}) };
}

export function request(input, options, callback) {
  if (typeof options === "function") callback = options;
  const settings = requestOptions(input, options);
  return new ClientRequest(requestUrl(input, settings), settings, callback);
}

export function get(input, options, callback) {
  const requestObject = request(input, options, callback);
  requestObject.end();
  return requestObject;
}

export function createServer(options, requestListener) { return new Server(options, requestListener); }

export const METHODS = ["ACL", "BIND", "CHECKOUT", "CONNECT", "COPY", "DELETE", "GET", "HEAD", "LINK", "LOCK", "M-SEARCH", "MERGE", "MKACTIVITY", "MKCALENDAR", "MKCOL", "MOVE", "NOTIFY", "OPTIONS", "PATCH", "POST", "PRI", "PROPFIND", "PROPPATCH", "PURGE", "PUT", "QUERY", "REBIND", "REPORT", "SEARCH", "SOURCE", "SUBSCRIBE", "TRACE", "UNBIND", "UNLINK", "UNLOCK", "UNSUBSCRIBE"];
export const STATUS_CODES = {
  100: "Continue", 101: "Switching Protocols", 102: "Processing", 200: "OK", 201: "Created", 202: "Accepted", 203: "Non-Authoritative Information", 204: "No Content", 205: "Reset Content", 206: "Partial Content", 207: "Multi-Status", 208: "Already Reported", 226: "IM Used", 300: "Multiple Choices", 301: "Moved Permanently", 302: "Found", 303: "See Other", 304: "Not Modified", 305: "Use Proxy", 307: "Temporary Redirect", 308: "Permanent Redirect", 400: "Bad Request", 401: "Unauthorized", 402: "Payment Required", 403: "Forbidden", 404: "Not Found", 405: "Method Not Allowed", 406: "Not Acceptable", 407: "Proxy Authentication Required", 408: "Request Timeout", 409: "Conflict", 410: "Gone", 411: "Length Required", 412: "Precondition Failed", 413: "Payload Too Large", 414: "URI Too Long", 415: "Unsupported Media Type", 416: "Range Not Satisfiable", 417: "Expectation Failed", 418: "I'm a teapot", 421: "Misdirected Request", 422: "Unprocessable Entity", 423: "Locked", 424: "Failed Dependency", 425: "Too Early", 426: "Upgrade Required", 428: "Precondition Required", 429: "Too Many Requests", 431: "Request Header Fields Too Large", 451: "Unavailable For Legal Reasons", 500: "Internal Server Error", 501: "Not Implemented", 502: "Bad Gateway", 503: "Service Unavailable", 504: "Gateway Timeout", 505: "HTTP Version Not Supported", 506: "Variant Also Negotiates", 507: "Insufficient Storage", 508: "Loop Detected", 509: "Bandwidth Limit Exceeded", 510: "Not Extended", 511: "Network Authentication Required",
};
export const globalAgent = new Agent();
export const maxHeaderSize = 16 * 1024;
export const _connectionListener = () => {};
export function setMaxIdleHTTPParsers() {}
export function validateHeaderName(name) {
  if (!/^[!#$%&'*+.^_`|~0-9A-Za-z-]+$/.test(String(name))) throw new TypeError(`Invalid header name: ${name}`);
  return name;
}
export function validateHeaderValue(name, value) {
  if (/\r|\n/.test(String(value))) throw new TypeError(`Invalid value for header ${name}`);
  return value;
}

export { CloseEvent, MessageEvent, WebSocket };
export default {
  Agent, ClientRequest, CloseEvent, IncomingMessage, METHODS, MessageEvent, OutgoingMessage, STATUS_CODES, Server, ServerResponse,
  WebSocket, _connectionListener, createServer, get, globalAgent, maxHeaderSize, request, setMaxIdleHTTPParsers,
  validateHeaderName, validateHeaderValue,
};
