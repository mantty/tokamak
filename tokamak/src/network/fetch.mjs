import { markHostObject } from "../globals/objects.mjs";
import { httpFetch, httpStatusText } from "tokamak:host";
import { randomUUID } from "../globals/crypto.mjs";
import { hostObjectCopies, uncloneable } from "../globals/structured-clone.mjs";
import { TextDecoder, TextEncoder } from "../streams/text.mjs";
import { ReadableStream, isDisturbed, setStreamLength } from "../streams/web.mjs";
import { ErrorEvent, Event, EventTarget, MessageEvent } from "../events/web.mjs";
import { AbortController, AbortSignal } from "../events/abort.mjs";
import { URL, URLSearchParams } from "./url.mjs";
import { nativeWebSocket } from "./websocket.mjs";

function string(value) {
  if (typeof value === "symbol") throw new TypeError("Cannot convert a Symbol to a string");
  return String(value);
}

function usvString(value) {
  const input = string(value);
  let output = "";
  for (let index = 0; index < input.length; index += 1) {
    const code = input.charCodeAt(index);
    if (code >= 0xd800 && code <= 0xdbff) {
      const next = input.charCodeAt(index + 1);
      if (next >= 0xdc00 && next <= 0xdfff) {
        output += input[index] + input[index + 1];
        index += 1;
      } else output += "\ufffd";
    } else if (code >= 0xdc00 && code <= 0xdfff) output += "\ufffd";
    else output += input[index];
  }
  return output;
}

// A copy of an ArrayBuffer's or view's bytes; undefined for any other value.
function bufferSourceBytes(value) {
  if (value instanceof ArrayBuffer) return new Uint8Array(value).slice();
  if (ArrayBuffer.isView(value)) return new Uint8Array(value.buffer, value.byteOffset, value.byteLength).slice();
}

function writeChunks(output, chunks) {
  let offset = 0;
  for (const chunk of chunks) {
    output.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return output;
}

function bytes(value) {
  if (value == null) return new Uint8Array();
  if (value instanceof Uint8Array) return value.slice();
  const copy = bufferSourceBytes(value);
  if (copy) return copy;
  if (value instanceof Blob) return blobBytes(value).slice();
  if (value instanceof URLSearchParams) return new TextEncoder().encode(value.toString());
  if (value instanceof FormData) return formDataBody(value).bytes;
  return new TextEncoder().encode(usvString(value));
}

function bodyStream(value) {
  const data = value instanceof Uint8Array ? value.slice() : bytes(value);
  const stream = new ReadableStream({ start(controller) {
    if (data.byteLength > 0) controller.enqueue(data.slice());
    controller.close();
  } });
  return setStreamLength(stream, data.byteLength);
}

function consumeStream(stream) {
  const reader = stream.getReader();
  const chunks = [];
  let length = 0;
  return (async () => {
    try {
      while (true) {
        const result = await reader.read();
        if (result.done) break;
        const chunk = bufferSourceBytes(result.value);
        if (!chunk) throw new TypeError("Response stream must contain bytes");
        chunks.push(chunk);
        length += chunk.byteLength;
      }
    } finally {
      reader.releaseLock();
    }
    return writeChunks(new Uint8Array(length), chunks);
  })();
}

// Reads `stream` for the host: `read()` gives each chunk as bytes, then null.
function hostStreamReader(stream) {
  const reader = stream.getReader();
  return {
    async read() {
      const { done, value } = await reader.read();
      if (done) return null;
      if (typeof value === "string") return new TextEncoder().encode(value);
      if (value instanceof ArrayBuffer) return new Uint8Array(value);
      if (ArrayBuffer.isView(value)) return new Uint8Array(value.buffer, value.byteOffset, value.byteLength);
      throw new TypeError("response stream chunks must be byte-oriented");
    },
    cancel: () => reader.cancel(),
  };
}

function bodyInitType(value) {
  if (typeof value === "string") return "text/plain;charset=UTF-8";
  if (value instanceof URLSearchParams) return "application/x-www-form-urlencoded;charset=UTF-8";
  if (value instanceof Blob && value.type !== "") return value.type;
  return null;
}

// A body init's content, as a stream or bytes, and the content type it implies.
function bodyInit(value) {
  if (value instanceof ReadableStream) return { bytes: null, stream: value, contentType: null };
  if (value instanceof FormData) return formDataBody(value);
  return { bytes: bytes(value), stream: null, contentType: bodyInitType(value) };
}

function validateHeaderName(name) {
  const value = string(name);
  if (!/^[!#$%&'*+.^_`|~0-9A-Za-z-]+$/.test(value)) throw new TypeError("Invalid header name");
  return value.toLowerCase();
}

function normalizeHeaderValue(value) {
  const normalized = string(value).replace(/^[\t ]+|[\t ]+$/g, "");
  if (/[\0-\x08\x0a-\x1f\x7f]/.test(normalized)) throw new TypeError("Invalid header value");
  return normalized;
}

export class Headers {
  #values = new Map();

  constructor(init) {
    markHostObject(this, "Headers");
    if (Object(init) === init && #values in init) {
      for (const [name, values] of init.#values) this.#values.set(name, values.slice());
    } else if (init != null && typeof init[Symbol.iterator] === "function") {
      for (const entry of init) {
        if (entry == null || typeof entry[Symbol.iterator] !== "function") throw new TypeError("Invalid header pair");
        const pair = [...entry];
        if (pair.length !== 2) throw new TypeError("Invalid header pair");
        this.append(pair[0], pair[1]);
      }
    } else if (init != null) {
      for (const name of Object.keys(Object(init))) this.append(name, init[name]);
    }
  }

  append(name, value) {
    const key = validateHeaderName(name);
    const next = normalizeHeaderValue(value);
    const values = this.#values.get(key) ?? [];
    if (key === "set-cookie" || values.length === 0) values.push(next);
    else values[0] += ", " + next;
    this.#values.set(key, values);
  }
  set(name, value) { this.#values.set(validateHeaderName(name), [normalizeHeaderValue(value)]); }
  get(name) { return this.#values.get(validateHeaderName(name))?.join(", ") ?? null; }
  getAll(name) { return this.#values.get(validateHeaderName(name))?.slice() ?? []; }
  has(name) { return this.#values.has(validateHeaderName(name)); }
  delete(name) { this.#values.delete(validateHeaderName(name)); }
  getSetCookie() { return this.#values.get("set-cookie")?.slice() ?? []; }
  entries() { return headerEntries(this.#values).values(); }
  keys() { return headerEntries(this.#values).map(entry => entry[0]).values(); }
  values() { return headerEntries(this.#values).map(entry => entry[1]).values(); }
  forEach(callback, thisArg) { for (const [name, value] of this) callback.call(thisArg, value, name, this); }
  [Symbol.iterator]() { return this.entries(); }
  get [Symbol.toStringTag]() { return "Headers"; }

  static {
    hostObjectCopies.set("Headers", headers => new Headers(headers));
  }
}

function headerEntries(values) {
  const entries = [];
  for (const [name, list] of [...values].sort(([left], [right]) => left < right ? -1 : left > right ? 1 : 0)) {
    if (name === "set-cookie") for (const value of list) entries.push([name, value]);
    else entries.push([name, list.join(", ")]);
  }
  return entries;
}

// The bytes a Blob holds.
let blobBytes;

export class Blob {
  #bytes;
  #type;

  constructor(parts = [], options = {}) {
    markHostObject(this, "Blob");
    if (parts == null || typeof parts[Symbol.iterator] !== "function") throw new TypeError("Blob parts must be iterable");
    const chunks = [...parts].map(blobPartBytes);
    this.#bytes = writeChunks(new Uint8Array(chunks.reduce((total, chunk) => total + chunk.byteLength, 0)), chunks);
    this.#type = mimeType(options?.type);
  }
  get size() { return this.#bytes.byteLength; }
  get type() { return this.#type; }
  async text() { return new TextDecoder().decode(this.#bytes); }
  async arrayBuffer() { return this.#bytes.slice().buffer; }
  async bytes() { return this.#bytes.slice(); }
  stream() { return bodyStream(this.#bytes); }
  slice(start = 0, end = this.size, contentType = "") {
    const first = relativeIndex(start, this.size);
    const last = relativeIndex(end, this.size);
    return new Blob([first < last ? this.#bytes.subarray(first, last) : new Uint8Array()], { type: contentType });
  }
  get [Symbol.toStringTag]() { return "Blob"; }

  static {
    blobBytes = blob => blob.#bytes;
    hostObjectCopies.set("Blob", blob => new Blob([blob.#bytes], { type: blob.#type }));
  }
}

export class File extends Blob {
  #name;
  #lastModified;

  constructor(parts, name, options = {}) {
    if (arguments.length < 2) throw new TypeError("File name is required");
    super(parts, options);
    markHostObject(this, "File");
    this.#name = usvString(name);
    const modified = Number(options?.lastModified === undefined ? Date.now() : options.lastModified);
    this.#lastModified = Number.isNaN(modified) ? 0 : modified;
  }
  get name() { return this.#name; }
  get lastModified() { return this.#lastModified; }
  get [Symbol.toStringTag]() { return "File"; }
}

function formDataValue(value, filename) {
  if (value instanceof File && filename === undefined) return value;
  if (value instanceof Blob) {
    const name = filename === undefined ? (value instanceof File ? value.name : "blob") : usvString(filename);
    return new File([value], name, { type: value.type });
  }
  return usvString(value);
}

export class FormData {
  #entries = [];

  constructor() { markHostObject(this); }
  append(name, value, filename) { this.#entries.push([usvString(name), formDataValue(value, filename)]); }
  set(name, value, filename) {
    const key = usvString(name);
    const item = formDataValue(value, filename);
    const index = this.#entries.findIndex(([entryName]) => entryName === key);
    if (index < 0) this.#entries.push([key, item]);
    else {
      this.#entries[index] = [key, item];
      this.#entries = this.#entries.filter(([entryName], entryIndex) => entryName !== key || entryIndex === index);
    }
  }
  get(name) { return this.#entries.find(([key]) => key === usvString(name))?.[1] ?? null; }
  getAll(name) { return this.#entries.filter(([key]) => key === usvString(name)).map(([, value]) => value); }
  has(name) { return this.#entries.some(([key]) => key === usvString(name)); }
  delete(name) { this.#entries = this.#entries.filter(([key]) => key !== usvString(name)); }
  entries() { return this.#entries.map(entry => entry.slice()).values(); }
  keys() { return this.#entries.map(([key]) => key).values(); }
  values() { return this.#entries.map(([, value]) => value).values(); }
  forEach(callback, thisArg) { for (const [key, value] of this) callback.call(thisArg, value, key, this); }
  [Symbol.iterator]() { return this.entries(); }
  get [Symbol.toStringTag]() { return "FormData"; }
}

// The content to send, `{ bytes, stream }`, after which the body is used.
let takeBody;
// The content for a copy of a body, teeing its stream so both read it.
let cloneBody;
// The stream a body exposes, or null.
let bodyOf;

export class Body {
  #bytes;
  #stream;
  #body;
  #used = false;
  #disturbed = false;

  constructor(bytes = null, stream = null) {
    if (new.target === Body) throw new TypeError("Illegal constructor");
    this.#bytes = bytes;
    this.#stream = stream;
    this.#body = stream ?? (bytes === null ? null : bodyStream(bytes));
  }

  get body() { return this.#body; }
  get bodyUsed() { return this.#used || this.#disturbed || isDisturbed(this.#body); }
  async arrayBuffer() { return (await this.#consume()).buffer; }
  async bytes() { return this.#consume(); }
  async blob() {
    const data = await this.#consume();
    return new Blob([data], { type: this.headers.get("content-type") ?? "" });
  }
  async text() { return new TextDecoder().decode(await this.#consume()); }
  async json() { return JSON.parse(await this.text()); }
  async formData() {
    const contentType = this.headers.get("content-type") ?? "";
    const data = await this.#consume();
    const mediaType = contentType.split(";", 1)[0].trim().toLowerCase();
    if (mediaType === "application/x-www-form-urlencoded") return parseUrlEncoded(new TextDecoder().decode(data));
    if (mediaType === "multipart/form-data") return parseMultipart(data, contentType);
    throw new TypeError("Request body is not form data");
  }
  get [Symbol.toStringTag]() { return "Body"; }

  async #consume() {
    if (this.#used || this.#body?.locked) throw new TypeError("Body has already been used");
    if (this.#body === null) return new Uint8Array();
    this.#used = true;
    return consumeStream(this.#body);
  }

  static {
    takeBody = target => {
      if (target.#body !== null) target.#used = true;
      return { bytes: target.#bytes, stream: target.#body };
    };
    cloneBody = target => {
      if (target.#used || target.#body?.locked) throw new TypeError("Body has already been used");
      const disturbed = isDisturbed(target.#body);
      if (target.#stream === null && !disturbed) return target.#bytes?.slice() ?? null;
      const [first, second] = target.#body.tee();
      target.#body = first;
      if (target.#stream !== null) target.#stream = first;
      target.#disturbed ||= disturbed;
      return second;
    };
    bodyOf = target => target.#body;
  }
}

function requestMethod(value) {
  if (value === null) throw new TypeError("Invalid request method");
  const method = string(value);
  if (!/^[!#$%&'*+.^_`|~0-9A-Za-z-]+$/.test(method)) throw new TypeError("Invalid request method");
  const upper = method.toUpperCase();
  return ["DELETE", "GET", "HEAD", "OPTIONS", "POST", "PUT"].includes(upper) ? upper : method;
}

function requestRedirect(value) {
  if (value === null) throw new TypeError("Invalid redirect mode");
  const redirect = string(value).toLowerCase();
  if (redirect === "error") throw new TypeError('Invalid redirect value, must be one of "follow" or "manual" ("error" won\'t be implemented since it does not make sense at the edge; use "manual" and check the response status code).');
  if (!["error", "follow", "manual"].includes(redirect)) throw new TypeError("Invalid redirect mode");
  return redirect;
}

function transferBody(stream) {
  const reader = stream.getReader();
  return new ReadableStream({
    type: "bytes",
    async pull(controller) {
      try {
        const { done, value } = await reader.read();
        if (done) { reader.releaseLock(); controller.close(); }
        else controller.enqueue(value);
      } catch (error) {
        reader.releaseLock();
        controller.error(error);
      }
    },
    async cancel(reason) {
      try { await reader.cancel(reason); }
      finally { reader.releaseLock(); }
    },
  });
}

export class Request extends Body {
  #url;
  #method;
  #headers;
  #cache;
  #cf;
  #fetcher;
  #integrity;
  #keepalive;
  #redirect;
  #signal;
  #signalProvided;

  constructor(input, init = {}) {
    init = init ?? {};
    const source = input instanceof Request ? input : null;
    const hasBody = init.body !== undefined;
    if (source && !hasBody && (source.bodyUsed || source.body?.locked)) throw new TypeError("Cannot construct a Request from a used body");
    const method = requestMethod(init.method === undefined ? source?.method ?? "GET" : init.method);
    let content = { bytes: null, stream: null, contentType: null };
    if (!hasBody && source?.body != null) {
      const taken = takeBody(source);
      content = { bytes: taken.bytes?.slice() ?? null, stream: transferBody(taken.stream), contentType: null };
    }
    const inputBody = hasBody ? init.body : null;
    if ((method === "GET" || method === "HEAD") && (inputBody != null || content.stream !== null)) throw new TypeError("Request with GET/HEAD method cannot have body");
    if (inputBody != null) content = bodyInit(inputBody);
    const url = source?.url ?? new URL(input).href;
    const headers = new Headers(init.headers !== undefined ? init.headers : source?.headers);
    if (content.contentType && !headers.has("content-type")) headers.set("content-type", content.contentType);
    super(content.bytes, content.stream);
    markHostObject(this, "Request");
    this.#url = url;
    this.#method = method;
    this.#headers = headers;
    this.#cache = init.cache ?? source?.cache;
    this.#cf = init.cf ?? source?.cf;
    this.#fetcher = init.fetcher ?? source?.fetcher;
    if (init.integrity === null) throw new TypeError("Invalid integrity");
    this.#integrity = init.integrity === undefined ? source?.integrity ?? "" : init.integrity;
    this.#keepalive = Boolean(init.keepalive ?? source?.keepalive ?? false);
    this.#redirect = requestRedirect(init.redirect === undefined ? source?.redirect ?? "follow" : init.redirect);
    const inheritedSignal = init.signal === undefined ? source?.signal : init.signal;
    if (inheritedSignal != null && !(inheritedSignal instanceof AbortSignal)) throw new TypeError("Invalid signal");
    this.#signal = inheritedSignal ?? new AbortController().signal;
    this.#signalProvided = init.signal === undefined ? source?.#signalProvided ?? false : init.signal != null;
  }

  get cache() { return this.#cache; }
  get cf() { return this.#cf; }
  get fetcher() { return this.#fetcher; }
  get headers() { return this.#headers; }
  get integrity() { return this.#integrity; }
  get keepalive() { return this.#keepalive; }
  get method() { return this.#method; }
  get redirect() { return this.#redirect; }
  get signal() { return this.#signal; }
  get url() { return this.#url; }
  get [Symbol.toStringTag]() { return "Request"; }

  clone() {
    if (this.bodyUsed) throw new TypeError("Body has already been used");
    return new Request(this.#url, {
      method: this.#method,
      headers: this.#headers,
      body: cloneBody(this),
      cache: this.#cache,
      integrity: this.#integrity,
      keepalive: this.#keepalive,
      redirect: this.#redirect,
      cf: this.#cf,
      fetcher: this.#fetcher,
    });
  }

  static {
    hostObjectCopies.set("Request", (request, clone, keep) => {
      if (bodyOf(request) !== null || request.#signalProvided) throw uncloneable();
      const copy = keep(new Request(request.#url, { method: request.#method, headers: request.#headers, redirect: request.#redirect, cache: request.#cache }));
      copy.#cf = clone(request.#cf);
      return copy;
    });
  }
}

// The Request a Worker receives for the host's request descriptor and body bytes.
export function hostRequest(request, body) {
  return new Request(request.url, { method: request.method, headers: request.headers, body });
}

// The status line, headers, encoding, body (bytes or a host stream reader) and native WebSocket
// end the host sends for a Worker's response.
let hostResponse;

export class Response extends Body {
  #status;
  #statusText;
  #encodeBody;
  #headers;
  #redirected;
  #type;
  #url;
  #cf;
  #webSocket;

  constructor(body = null, init = {}) {
    init = init ?? {};
    const inputStatus = init.status;
    const status = Math.trunc(Number(inputStatus === undefined ? 200 : inputStatus));
    const webSocket = init.webSocket ?? null;
    if (webSocket !== null) {
      if (status !== 101) throw new RangeError("Responses with a WebSocket must have status code 101.");
    } else if (!Number.isInteger(status) || status < 200 || status > 599) {
      throw new RangeError("Invalid response status code");
    }
    if ([204, 205, 304].includes(status) && body !== null && body !== undefined) throw new TypeError("Response with null body status cannot have a body");
    const inputStatusText = init.statusText;
    const statusText = string(inputStatusText === undefined ? httpStatusText(status) : inputStatusText);
    if (/[\0-\x1f\x7f]/.test(statusText)) throw new TypeError("Invalid response status text");
    const inputEncoding = init instanceof Response ? init.#encodeBody : init.encodeBody;
    const encodeBody = inputEncoding === undefined ? "automatic" : string(inputEncoding);
    if (encodeBody !== "automatic" && encodeBody !== "manual") throw new TypeError(`encodeBody: unexpected value: ${encodeBody}`);
    const headers = new Headers(init.headers);
    const content = body == null ? { bytes: null, stream: null, contentType: null } : bodyInit(body);
    if (content.contentType && !headers.has("content-type")) headers.set("content-type", content.contentType);
    super(content.bytes, content.stream);
    markHostObject(this, "Response");
    this.#status = status;
    this.#statusText = statusText;
    this.#encodeBody = encodeBody;
    this.#headers = headers;
    this.#redirected = Boolean(init.redirected);
    this.#type = init.type ?? "default";
    this.#url = string(init.url ?? "");
    this.#cf = init.cf;
    this.#webSocket = webSocket;
  }

  get cf() { return this.#cf; }
  get headers() { return this.#headers; }
  get ok() { return this.#status >= 200 && this.#status < 300; }
  get redirected() { return this.#redirected; }
  get status() { return this.#status; }
  get statusText() { return this.#statusText; }
  get type() { return this.#type; }
  get url() { return this.#url; }
  get webSocket() { return this.#webSocket; }
  get [Symbol.toStringTag]() { return "Response"; }

  clone() {
    if (this.#status === 0) return Response.error();
    return new Response(cloneBody(this), {
      status: this.#status,
      statusText: this.#statusText,
      headers: this.#headers,
      url: this.#url,
      redirected: this.#redirected,
      type: this.#type,
      cf: this.#cf,
      webSocket: this.#webSocket,
    });
  }

  static error() {
    const response = new Response(null);
    response.#status = 0;
    response.#statusText = "";
    response.#type = "error";
    return response;
  }
  static json(data, init = {}) {
    init = init ?? {};
    const headers = new Headers(init.headers);
    if (!headers.has("content-type")) headers.set("content-type", "application/json");
    const body = JSON.stringify(data);
    return new Response(body === undefined ? "undefined" : body, { ...init, headers });
  }
  static redirect(url, status = 302) {
    const code = Math.trunc(Number(status));
    if (![301, 302, 303, 307, 308].includes(code)) throw new RangeError("Invalid redirect status code");
    return new Response(null, { status: code, headers: { location: new URL(url).href } });
  }

  static {
    hostObjectCopies.set("Response", (response, clone, keep) => {
      if (bodyOf(response) !== null || response.#webSocket !== null) throw uncloneable();
      const copy = keep(response.#status === 0 ? Response.error() : new Response(null, { status: response.#status, statusText: response.#statusText, headers: response.#headers }));
      copy.#cf = clone(response.#cf);
      return copy;
    });
    hostResponse = response => {
      if (!(#status in Object(response))) throw new TypeError("Incorrect type for Promise: the Promise did not resolve to 'Response'.");
      const { bytes, stream } = takeBody(response);
      return {
        status: response.#status,
        statusText: response.#statusText,
        headers: [...response.#headers],
        encodeBody: response.#encodeBody,
        body: bytes ?? (stream === null ? new Uint8Array() : hostStreamReader(stream)),
        webSocket: response.#webSocket === null ? null : nativeWebSocket(response.#webSocket),
      };
    };
  }
}

function relativeIndex(value, length) {
  const number = Number(value);
  if (Number.isNaN(number) || number === -Infinity) return 0;
  if (number === Infinity) return length;
  return number < 0 ? Math.max(length + Math.trunc(number), 0) : Math.min(Math.trunc(number), length);
}

function blobPartBytes(value) {
  if (value instanceof Blob) return blobBytes(value).slice();
  if (typeof value === "string") return new TextEncoder().encode(value);
  return bufferSourceBytes(value) ?? new TextEncoder().encode(usvString(value));
}

function mimeType(value) {
  const type = string(value === undefined ? "" : value);
  return /^[\x20-\x7e]*$/.test(type) ? type.toLowerCase() : "";
}

function parseUrlEncoded(value) {
  const form = new FormData();
  for (const pair of value.split("&")) {
    if (pair === "") continue;
    const separator = pair.indexOf("=");
    const key = decodeFormComponent(separator < 0 ? pair : pair.slice(0, separator));
    const item = decodeFormComponent(separator < 0 ? "" : pair.slice(separator + 1));
    form.append(key, item);
  }
  return form;
}

function decodeFormComponent(value) {
  const input = string(value).replace(/\+/g, " ");
  const encoder = new TextEncoder();
  const bytes = [];
  for (let index = 0; index < input.length;) {
    const first = input.charCodeAt(index + 1);
    const second = input.charCodeAt(index + 2);
    if (input[index] === "%" && index + 2 < input.length && hexDigit(first) >= 0 && hexDigit(second) >= 0) {
      bytes.push(hexDigit(first) * 16 + hexDigit(second));
      index += 3;
    } else if (input[index] === "%") index += 1;
    else {
      const character = String.fromCodePoint(input.codePointAt(index));
      bytes.push(...encoder.encode(character));
      index += character.length;
    }
  }
  return new TextDecoder().decode(new Uint8Array(bytes));
}

function hexDigit(code) {
  return code >= 48 && code <= 57 ? code - 48
    : code >= 65 && code <= 70 ? code - 55
      : code >= 97 && code <= 102 ? code - 87 : -1;
}

function parseMultipart(data, contentType) {
  const boundary = multipartBoundary(contentType);
  const delimiter = new TextEncoder().encode("--" + boundary);
  if (!matchesBytes(data, delimiter, 0)) throw new TypeError("Invalid multipart body");
  const form = new FormData();
  let position = 0;
  while (true) {
    const afterBoundary = position + delimiter.length;
    if (data[afterBoundary] === 45 && data[afterBoundary + 1] === 45) return form;
    if (data[afterBoundary] !== 13 || data[afterBoundary + 1] !== 10) throw new TypeError("Invalid multipart body");
    const headerStart = afterBoundary + 2;
    const headerEnd = indexOfBytes(data, new Uint8Array([13, 10, 13, 10]), headerStart);
    if (headerEnd < 0) throw new TypeError("Invalid multipart headers");
    const headers = parseMultipartHeaders(ascii(data.subarray(headerStart, headerEnd)));
    const contentStart = headerEnd + 4;
    const nextBoundary = nextMultipartBoundary(data, delimiter, contentStart);
    if (nextBoundary < 0) throw new TypeError("Invalid multipart body");
    const value = data.subarray(contentStart, nextBoundary - 2);
    const disposition = multipartDisposition(headers["content-disposition"]);
    if (disposition.filename !== undefined) {
      const type = headers["content-type"]?.split(";", 1)[0].trim() || "application/octet-stream";
      form.append(disposition.name, new File([value], disposition.filename, { type }));
    } else form.append(disposition.name, new TextDecoder().decode(value));
    position = nextBoundary;
  }
}

function multipartBoundary(contentType) {
  const match = /(?:^|;)\s*boundary\s*=\s*(?:"((?:\\.|[^"])*)"|([^;\s]*))/i.exec(contentType);
  if (!match) throw new TypeError("Multipart boundary is missing");
  const boundary = (match[1] ?? match[2]).replace(/\\(.)/g, "$1");
  if (boundary === "") throw new TypeError("Multipart boundary is empty");
  return boundary;
}

function parseMultipartHeaders(value) {
  const headers = {};
  for (const line of value.split("\r\n")) {
    const separator = line.indexOf(":");
    if (separator <= 0) throw new TypeError("Invalid multipart headers");
    headers[line.slice(0, separator).trim().toLowerCase()] = line.slice(separator + 1).trim();
  }
  return headers;
}

function multipartDisposition(value) {
  const input = string(value ?? "");
  const prefix = /^form-data\b/i.exec(input);
  if (!prefix) throw new TypeError("Invalid multipart disposition");
  let rest = input.slice(prefix[0].length);
  let name;
  let filename;
  while (rest.trim() !== "") {
    rest = rest.trimStart();
    if (!rest.startsWith(";")) throw new TypeError("Invalid multipart disposition");
    rest = rest.slice(1);
    const match = /^\s*([!#$%&'*+.^_`|~0-9A-Za-z-]+)\s*=\s*(?:"((?:\\.|[^"\\])*)"|([^;\s]*))/.exec(rest);
    if (!match) throw new TypeError("Invalid multipart disposition");
    const key = match[1].toLowerCase();
    if (key === "name") {
      if (match[2] === undefined) throw new TypeError("Invalid multipart disposition");
      name = match[2].replace(/\\(.)/g, "$1");
    } else if (key === "filename" && match[2] !== undefined) filename = match[2].replace(/\\(.)/g, "$1");
    rest = rest.slice(match[0].length);
  }
  if (name === undefined) throw new TypeError("Invalid multipart disposition");
  return { name, filename };
}

function nextMultipartBoundary(data, delimiter, start) {
  let position = indexOfBytes(data, delimiter, start);
  while (position >= 0) {
    if (position >= 2 && data[position - 2] === 13 && data[position - 1] === 10) return position;
    position = indexOfBytes(data, delimiter, position + 1);
  }
  return -1;
}

function matchesBytes(haystack, needle, start) {
  if (start < 0 || start + needle.length > haystack.length) return false;
  for (let offset = 0; offset < needle.length; offset += 1) if (haystack[start + offset] !== needle[offset]) return false;
  return true;
}

function indexOfBytes(haystack, needle, start = 0) {
  for (let index = start; index <= haystack.length - needle.length; index += 1) {
    if (matchesBytes(haystack, needle, index)) return index;
  }
  return -1;
}

function ascii(bytes) {
  let value = "";
  for (const byte of bytes) value += String.fromCharCode(byte);
  return value;
}

function formDataBody(form) {
  const boundary = "----tokamak-" + randomUUID();
  const encoder = new TextEncoder();
  const chunks = [];
  for (const [name, value] of form) {
    const file = value instanceof File;
    chunks.push(encoder.encode("--" + boundary + "\r\nContent-Disposition: form-data; name=\"" + quoteHeader(name) + "\""
      + (file ? "; filename=\"" + quoteHeader(value.name) + "\"" : "")
      + (file ? "\r\nContent-Type: " + (value.type || "application/octet-stream") : "") + "\r\n\r\n"));
    chunks.push(file ? blobBytes(value) : encoder.encode(value), encoder.encode("\r\n"));
  }
  chunks.push(encoder.encode("--" + boundary + "--\r\n"));
  const body = writeChunks(new Uint8Array(chunks.reduce((total, value) => total + value.byteLength, 0)), chunks);
  return { bytes: body, stream: null, contentType: "multipart/form-data; boundary=" + boundary };
}

function quoteHeader(value) { return string(value).replace(/["\\\r\n]/g, character => "\\" + character); }


export async function fetch(input, init) {
  const request = new Request(input, init);
  if (request.signal.aborted) throw request.signal.reason;
  const { bytes, stream } = takeBody(request);
  const task = httpFetch(
    request.url,
    request.method,
    JSON.stringify([...request.headers]),
    bytes ?? stream,
    request.redirect,
  );
  globalThis.__tokamak_context.waitUntil(task.upload);
  const abort = () => task.cancel();
  request.signal.addEventListener("abort", abort, { once: true });
  const finish = () => request.signal.removeEventListener("abort", abort);
  let response;
  try {
    response = await task.response;
    if (response.bodyless) await task.upload;
  } catch (error) {
    finish();
    if (request.signal.aborted) throw fetchAbortReason(request.signal);
    throw error;
  }
  if (request.signal.aborted) { task.cancel(); finish(); throw fetchAbortReason(request.signal); }
  const body = response.bodyless ? null : new ReadableStream({
    type: "bytes",
    async pull(controller) {
      try {
        request.signal.throwIfAborted();
        const chunk = await response.read();
        request.signal.throwIfAborted();
        if (chunk == null) { finish(); controller.close(); }
        else controller.enqueue(chunk);
      } catch (error) {
        task.cancel(); finish();
        controller.error(request.signal.aborted ? fetchAbortReason(request.signal) : error);
      }
    },
    cancel() { task.cancel(); finish(); },
  });
  if (response.bodyless) finish();
  if (body !== null && response.length != null) setStreamLength(body, response.length);
  return new Response(body, {
    status: response.status,
    statusText: response.statusText,
    headers: JSON.parse(response.headers),
    url: response.url,
    redirected: response.redirected,
  });
}

function fetchAbortReason(signal) {
  return signal.reason instanceof Error ? signal.reason : new Error(String(signal.reason));
}

export class Cache {
  #name;
  constructor(name = "default") { markHostObject(this); this.#name = string(name); }
  async match(request, options = {}) {
    if (!options.ignoreMethod && requestMethodForCache(request) !== "GET") return undefined;
    const entry = await cacheHost("cacheMatch", [this.#name, cacheKey(request, options)]);
    if (entry === null || entry === undefined) return undefined;
    return new Response(entry.body, {
      status: entry.status,
      statusText: entry.statusText,
      headers: JSON.parse(entry.headers),
      url: entry.url,
      redirected: entry.redirected,
      type: entry.type,
    });
  }
  async matchAll(request, options = {}) { const value = await this.match(request, options); return value ? [value] : []; }
  async put(request, response) {
    if (requestMethodForCache(request) !== "GET") throw new TypeError("Cache.put only accepts GET requests");
    if (!(response instanceof Response)) throw new TypeError("Cache.put requires a Response");
    const copy = response.clone();
    await cacheHost("cachePut", [
      this.#name, cacheKey(request), {
        status: copy.status,
        statusText: copy.statusText,
        headers: JSON.stringify([...copy.headers]),
        url: copy.url,
        redirected: copy.redirected,
        type: copy.type,
      }, new Uint8Array(await copy.arrayBuffer()),
    ]);
  }
  async delete(request, options = {}) {
    if (!options.ignoreMethod && requestMethodForCache(request) !== "GET") return false;
    return cacheHost("cacheDelete", [this.#name, cacheKey(request, options)]);
  }
  async keys() { throw cacheNotImplemented("Cache", "keys"); }
  async add() { throw cacheNotImplemented("Cache", "add"); }
  async addAll() { throw cacheNotImplemented("Cache", "addAll"); }
}

export class CacheStorage {
  #caches;
  constructor() {
    markHostObject(this);
    this.default = new Cache();
    this.#caches = new Map([["default", this.default]]);
  }
  async open(name = "default") {
    const key = string(name);
    let cache = this.#caches.get(key);
    if (!cache) {
      cache = new Cache(key);
      this.#caches.set(key, cache);
    }
    return cache;
  }
  async delete() { throw cacheNotImplemented("CacheStorage", "delete"); }
  async has() { throw cacheNotImplemented("CacheStorage", "has"); }
  async keys() { throw cacheNotImplemented("CacheStorage", "keys"); }
  async match() { throw cacheNotImplemented("CacheStorage", "match"); }
}

function requestMethodForCache(request) { return request instanceof Request ? request.method : "GET"; }
function cacheKey(request, options = {}) {
  const url = new URL(request instanceof Request ? request.url : request);
  if (options.ignoreSearch) throw new Error("The 'ignoreSearch' field on 'CacheQueryOptions' is not implemented.");
  return url.href;
}
async function cacheHost(name, args) { return (await import("tokamak:host"))[name](...args); }
function cacheNotImplemented(type, method) { return new Error(`Failed to execute '${method}' on '${type}': the method is not implemented.`); }

const eventSourceStream = Symbol("event-source-stream");

export class EventSource extends EventTarget {
  #url;
  #withCredentials;
  #readyState = EventSource.CONNECTING;
  #onopen = null;
  #onmessage = null;
  #onerror = null;
  #reader = null;
  #controller = new AbortController();
  #lastEventId = "";

  constructor(url, options = {}) {
    super();
    options ??= {};
    const stream = options[eventSourceStream];
    this.#url = stream ? "" : new URL(url, globalThis.__tokamak_request?.url).href;
    this.#withCredentials = Boolean(options.withCredentials);
    if (stream) {
      this.#readyState = EventSource.OPEN;
      void this.#consume(stream);
    }
    else void this.#connect(options);
  }

  get url() { return this.#url; }
  get readyState() { return this.#readyState; }
  get withCredentials() { return this.#withCredentials; }
  get onopen() { return this.#onopen; }
  set onopen(value) { this.#onopen = value; }
  get onmessage() { return this.#onmessage; }
  set onmessage(value) { this.#onmessage = value; }
  get onerror() { return this.#onerror; }
  set onerror(value) { this.#onerror = value; }

  close() {
    if (this.#readyState === EventSource.CLOSED) return;
    this.#readyState = EventSource.CLOSED;
    this.#controller.abort();
    void this.#reader?.cancel().catch(() => {});
  }

  static from(stream) {
    if (!(stream instanceof ReadableStream)) throw new TypeError("EventSource.from requires a ReadableStream");
    return new EventSource("", { [eventSourceStream]: stream });
  }

  #emit(event) {
    this.dispatchEvent(event);
    const handler = this["on" + event.type];
    if (typeof handler === "function") handler.call(this, event);
  }

  #fail(error) {
    if (this.#readyState === EventSource.CLOSED) return;
    this.#readyState = EventSource.CONNECTING;
    this.#emit(new ErrorEvent("error", { error, message: error?.message ?? String(error) }));
  }

  async #connect(options) {
    try {
      const fetcher = options.fetcher ?? globalThis;
      if (typeof fetcher.fetch !== "function") throw new TypeError("EventSource fetcher must provide fetch()");
      const response = await fetcher.fetch.call(fetcher, this.#url, {
        headers: { accept: "text/event-stream" },
        signal: this.#controller.signal,
      });
      if (!response?.ok || response.body === null) throw new TypeError("EventSource response was not a stream");
      this.#readyState = EventSource.OPEN;
      this.#emit(new Event("open"));
      await this.#consume(response.body);
    } catch (error) {
      this.#fail(error);
    }
  }

  async #consume(stream) {
    const reader = stream.getReader();
    this.#reader = reader;
    const decoder = new TextDecoder();
    let input = "";
    let data = [];
    let eventName = "";
    const dispatch = () => {
      if (data.length === 0) return;
      this.#emit(new MessageEvent(eventName || "message", {
        data: data.join("\n"),
        lastEventId: this.#lastEventId,
      }));
      data = [];
      eventName = "";
    };
    const line = value => {
      if (value === "") {
        dispatch();
        return;
      }
      if (value[0] === ":") return;
      const separator = value.indexOf(":");
      const field = separator < 0 ? value : value.slice(0, separator);
      let fieldValue = separator < 0 ? "" : value.slice(separator + 1);
      if (fieldValue.startsWith(" ")) fieldValue = fieldValue.slice(1);
      if (field === "data") data.push(fieldValue);
      else if (field === "event") eventName = fieldValue;
      else if (field === "id" && !fieldValue.includes("\0")) this.#lastEventId = fieldValue;
    };
    try {
      while (true) {
        const result = await reader.read();
        if (result.done) break;
        const lines = (input + decoder.decode(result.value, { stream: true })).split("\n");
        input = lines.pop();
        for (const value of lines) line(value.endsWith("\r") ? value.slice(0, -1) : value);
      }
      input += decoder.decode();
      if (input !== "") line(input.endsWith("\r") ? input.slice(0, -1) : input);
      dispatch();
      this.#readyState = EventSource.CLOSED;
    } catch (error) {
      this.#fail(error);
    } finally {
      if (this.#reader === reader) this.#reader = null;
    }
  }
}
for (const [value, name] of ["CONNECTING", "OPEN", "CLOSED"].entries()) {
  EventSource[name] = value;
  EventSource.prototype[name] = value;
}

export { bodyStream, bytes, consumeStream, hostResponse };
