import { TextDecoder, TextEncoder } from "../streams/text.mjs";
import { ReadableStream } from "../streams/web.mjs";

function bodyBytes(value) {
  if (value == null) return new Uint8Array();
  if (value instanceof ArrayBuffer) return new Uint8Array(value).slice();
  if (ArrayBuffer.isView(value)) return new Uint8Array(value.buffer, value.byteOffset, value.byteLength).slice();
  if (value instanceof Blob) return value.__bytes.slice();
  return new TextEncoder().encode(String(value));
}

export class Headers {
  constructor(init) {
    this.__values = new Map();
    if (init instanceof Headers) {
      for (const [name, value] of init) this.set(name, value);
    } else if (Array.isArray(init)) {
      for (const [name, value] of init) this.append(name, value);
    } else if (init) {
      for (const name of Object.keys(init)) this.append(name, init[name]);
    }
  }
  append(name, value) {
    const key = String(name).toLowerCase();
    const next = String(value);
    this.__values.set(key, this.__values.has(key) ? `${this.__values.get(key)}, ${next}` : next);
  }
  set(name, value) { this.__values.set(String(name).toLowerCase(), String(value)); }
  get(name) { return this.__values.get(String(name).toLowerCase()) ?? null; }
  has(name) { return this.__values.has(String(name).toLowerCase()); }
  delete(name) { this.__values.delete(String(name).toLowerCase()); }
  entries() { return this.__values.entries(); }
  keys() { return this.__values.keys(); }
  values() { return this.__values.values(); }
  forEach(callback, thisArg) { this.__values.forEach((value, name) => callback.call(thisArg, value, name, this)); }
  getSetCookie() { const value = this.get("set-cookie"); return value ? value.split(/, (?=[^;]+=)/) : []; }
  [Symbol.iterator]() { return this.entries(); }
}

export class Request {
  constructor(input, init = {}) {
    if (input instanceof Request) {
      this.url = input.url;
      this.method = input.method;
      this.headers = new Headers(input.headers);
      this.__body = input.__body;
    } else {
      this.url = String(input);
      this.method = String(init.method ?? "GET").toUpperCase();
      this.headers = new Headers(init.headers);
      this.__body = init.body == null ? null : bodyBytes(init.body);
    }
    this.bodyUsed = false;
    this.redirect = init.redirect ?? "follow";
    this.signal = init.signal ?? { aborted: false };
    this.cf = init.cf;
  }
  get body() { return this.__body == null ? null : { __tokamak_body: this.__body }; }
  async text() { this.bodyUsed = true; return this.__body instanceof Uint8Array ? new TextDecoder().decode(this.__body) : this.__body ?? ""; }
  async json() { return JSON.parse(await this.text()); }
  async arrayBuffer() { this.bodyUsed = true; return bodyBytes(this.__body).buffer; }
  clone() { return new Request(this); }
}

export class Response {
  constructor(body = null, init = {}) {
    if (body instanceof Response) {
      init = { ...body, headers: new Headers(body.headers) };
      body = body.__stream ?? body.__body;
    }
    this.status = init.status ?? 200;
    this.statusText = init.statusText ?? "";
    this.headers = new Headers(init.headers);
    this.__stream = body instanceof ReadableStream ? body : null;
    this.__body = this.__stream || body == null ? null : bodyBytes(body);
    this.__tokamak_body = this.__body;
    this.body = this.__stream ?? (this.__body == null ? null : { __tokamak_body: this.__body });
    this.ok = this.status >= 200 && this.status < 300;
    this.redirected = false;
    this.type = "default";
    this.url = "";
    this.webSocket = init.webSocket;
  }
  async text() { return new TextDecoder().decode(await this.arrayBuffer()); }
  async arrayBuffer() {
    if (!this.__stream) return bodyBytes(this.__body).buffer;
    const reader = this.__stream.getReader();
    const chunks = [];
    let length = 0;
    try {
      while (true) {
        const result = await reader.read();
        if (result.done) break;
        if (!(result.value instanceof ArrayBuffer) && !ArrayBuffer.isView(result.value)) throw new TypeError("Response stream must contain bytes");
        const chunk = bodyBytes(result.value);
        chunks.push(chunk);
        length += chunk.byteLength;
      }
    } finally {
      reader.releaseLock();
    }
    const bytes = new Uint8Array(length);
    let offset = 0;
    for (const chunk of chunks) { bytes.set(chunk, offset); offset += chunk.byteLength; }
    return bytes.buffer;
  }
  async json() { return JSON.parse(await this.text()); }
  clone() { return new Response(this.__body, { status: this.status, statusText: this.statusText, headers: this.headers }); }
  static error() { return new Response(null, { status: 0 }); }
  static redirect(url, status = 302) { return new Response(null, { status, headers: { location: url } }); }
}

export class Blob {
  constructor(parts = [], options = {}) {
    const chunks = parts.map((part) => {
      if (part instanceof Uint8Array) return new Uint8Array(part);
      if (part instanceof ArrayBuffer) return new Uint8Array(part);
      if (ArrayBuffer.isView(part)) return new Uint8Array(part.buffer, part.byteOffset, part.byteLength);
      return new TextEncoder().encode(String(part));
    });
    const length = chunks.reduce((total, chunk) => total + chunk.length, 0);
    this.__bytes = new Uint8Array(length);
    let offset = 0;
    for (const chunk of chunks) {
      this.__bytes.set(chunk, offset);
      offset += chunk.length;
    }
    this.type = options.type ?? "";
    this.size = this.__bytes.length;
  }
  async text() { return new TextDecoder().decode(this.__bytes); }
  async arrayBuffer() { return this.__bytes.slice().buffer; }
}
