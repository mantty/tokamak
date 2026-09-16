import { markHostObject } from "../globals/objects.mjs";
import { httpStatusText } from "tokamak:host";
import { TextDecoder, TextEncoder } from "../streams/text.mjs";
import { ReadableStream, isDisturbed } from "../streams/web.mjs";
import { ErrorEvent, Event, EventTarget, MessageEvent } from "../events/events.mjs";
import { blobBrand, URL, URLSearchParams } from "./url.mjs";

function string(value) {
  if (typeof value === "symbol") throw new TypeError("Cannot convert a Symbol to a string");
  return String(value);
}

function hidden(object, name, value) {
  Object.defineProperty(object, name, { configurable: true, enumerable: false, writable: true, value });
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

function bytes(value) {
  if (value == null) return new Uint8Array();
  if (value instanceof Uint8Array) return value.slice();
  if (value instanceof ArrayBuffer) return new Uint8Array(value).slice();
  if (ArrayBuffer.isView(value)) return new Uint8Array(value.buffer, value.byteOffset, value.byteLength).slice();
  if (value instanceof Blob) return value.__bytes.slice();
  if (value instanceof URLSearchParams) return new TextEncoder().encode(value.toString());
  if (value instanceof FormData) return formDataBody(value).body;
  return new TextEncoder().encode(usvString(value));
}

function streamChunkBytes(value) {
  if (value instanceof ArrayBuffer) return new Uint8Array(value).slice();
  if (ArrayBuffer.isView(value)) return new Uint8Array(value.buffer, value.byteOffset, value.byteLength).slice();
  throw new TypeError("Response stream must contain bytes");
}

function bodyStream(value, onUse) {
  const data = value instanceof Uint8Array ? value.slice() : bytes(value);
  const stream = new ReadableStream({ start(controller) {
    if (data.byteLength > 0) controller.enqueue(data.slice());
    controller.close();
  } });
  stream.__bodyBytes = data;
  return stream;
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
        const chunk = streamChunkBytes(result.value);
        chunks.push(chunk);
        length += chunk.byteLength;
      }
    } finally {
      reader.releaseLock();
    }
    const output = new Uint8Array(length);
    let offset = 0;
    for (const chunk of chunks) {
      output.set(chunk, offset);
      offset += chunk.byteLength;
    }
    return output;
  })();
}

function bodyInitType(value) {
  if (typeof value === "string") return "text/plain;charset=UTF-8";
  if (value instanceof URLSearchParams) return "application/x-www-form-urlencoded;charset=UTF-8";
  if (value instanceof Blob && value.type !== "") return value.type;
  return null;
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
  constructor(init) {
    markHostObject(this, "Headers");
    hidden(this, "__values", new Map());
    if (init instanceof Headers) {
      for (const [name, values] of init.__values) this.__values.set(name, values.slice());
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
    const values = this.__values.get(key) ?? [];
    if (key === "set-cookie" || values.length === 0) values.push(next);
    else values[0] += ", " + next;
    this.__values.set(key, values);
  }
  set(name, value) { this.__values.set(validateHeaderName(name), [normalizeHeaderValue(value)]); }
  get(name) { return this.__values.get(validateHeaderName(name))?.join(", ") ?? null; }
  getAll(name) { return this.__values.get(validateHeaderName(name))?.slice() ?? []; }
  has(name) { return this.__values.has(validateHeaderName(name)); }
  delete(name) { this.__values.delete(validateHeaderName(name)); }
  getSetCookie() { return this.__values.get("set-cookie")?.slice() ?? []; }
  entries() { return headerEntries(this).values(); }
  keys() { return headerEntries(this).map(entry => entry[0]).values(); }
  values() { return headerEntries(this).map(entry => entry[1]).values(); }
  forEach(callback, thisArg) { for (const [name, value] of this) callback.call(thisArg, value, name, this); }
  [Symbol.iterator]() { return this.entries(); }
  get [Symbol.toStringTag]() { return "Headers"; }
}

function headerEntries(headers) {
  const entries = [];
  for (const [name, values] of [...headers.__values].sort(([left], [right]) => left < right ? -1 : left > right ? 1 : 0)) {
    if (name === "set-cookie") for (const value of values) entries.push([name, value]);
    else entries.push([name, values.join(", ")]);
  }
  return entries;
}

export class Blob {
  constructor(parts = [], options = {}) {
    markHostObject(this, "Blob");
    if (parts == null || typeof parts[Symbol.iterator] !== "function") throw new TypeError("Blob parts must be iterable");
    const chunks = [...parts].map(blobPartBytes);
    const length = chunks.reduce((total, chunk) => total + chunk.byteLength, 0);
    hidden(this, "__bytes", new Uint8Array(length));
    let offset = 0;
    for (const chunk of chunks) {
      this.__bytes.set(chunk, offset);
      offset += chunk.byteLength;
    }
    hidden(this, blobBrand, true);
    hidden(this, "__type", mimeType(options?.type));
    hidden(this, "__size", this.__bytes.byteLength);
  }
  get size() { return this.__size; }
  get type() { return this.__type; }
  async text() { return new TextDecoder().decode(this.__bytes); }
  async arrayBuffer() { return this.__bytes.slice().buffer; }
  async bytes() { return this.__bytes.slice(); }
  stream() { return bodyStream(this.__bytes); }
  slice(start = 0, end = this.size, contentType = "") {
    const first = relativeIndex(start, this.size);
    const last = relativeIndex(end, this.size);
    return new Blob([first < last ? this.__bytes.subarray(first, last) : new Uint8Array()], { type: contentType });
  }
  get [Symbol.toStringTag]() { return "Blob"; }
}

export class File extends Blob {
  constructor(parts, name, options = {}) {
    if (arguments.length < 2) throw new TypeError("File name is required");
    super(parts, options);
    markHostObject(this, "File");
    hidden(this, "__name", usvString(name));
    const modified = Number(options?.lastModified === undefined ? Date.now() : options.lastModified);
    hidden(this, "__lastModified", Number.isNaN(modified) ? 0 : modified);
  }
  get name() { return this.__name; }
  get lastModified() { return this.__lastModified; }
  get [Symbol.toStringTag]() { return "File"; }
}

function formDataValue(value, filename) {
  if (value instanceof File && filename === undefined) return value;
  if (value instanceof Blob) {
    const name = filename === undefined ? (value instanceof File ? value.name : "blob") : usvString(filename);
    return new File([value.__bytes], name, { type: value.type });
  }
  return usvString(value);
}

export class FormData {
  constructor() { markHostObject(this); hidden(this, "__entries", []); }
  append(name, value, filename) { this.__entries.push([usvString(name), formDataValue(value, filename)]); }
  set(name, value, filename) {
    const key = usvString(name);
    const item = formDataValue(value, filename);
    const index = this.__entries.findIndex(([entryName]) => entryName === key);
    if (index < 0) this.__entries.push([key, item]);
    else {
      this.__entries[index] = [key, item];
      this.__entries = this.__entries.filter(([entryName], entryIndex) => entryName !== key || entryIndex === index);
    }
  }
  get(name) { return this.__entries.find(([key]) => key === usvString(name))?.[1] ?? null; }
  getAll(name) { return this.__entries.filter(([key]) => key === usvString(name)).map(([, value]) => value); }
  has(name) { return this.__entries.some(([key]) => key === usvString(name)); }
  delete(name) { this.__entries = this.__entries.filter(([key]) => key !== usvString(name)); }
  entries() { return this.__entries.map(entry => entry.slice()).values(); }
  keys() { return this.__entries.map(([key]) => key).values(); }
  values() { return this.__entries.map(([, value]) => value).values(); }
  forEach(callback, thisArg) { for (const [key, value] of this) callback.call(thisArg, value, key, this); }
  [Symbol.iterator]() { return this.entries(); }
  get [Symbol.toStringTag]() { return "FormData"; }
}

export class Body {
  get body() { return this.__bodyStream; }
  get bodyUsed() { return Boolean(this.__bodyConsumed || this.__bodyDisturbed || isDisturbed(this.__bodyStream)); }
  async arrayBuffer() { return (await consumeBody(this)).buffer; }
  async bytes() { return consumeBody(this); }
  async blob() {
    const data = await consumeBody(this);
    return new Blob([data], { type: this.headers.get("content-type") ?? "" });
  }
  async text() { return new TextDecoder().decode(await consumeBody(this)); }
  async json() { return JSON.parse(await this.text()); }
  async formData() {
    const contentType = this.headers.get("content-type") ?? "";
    const data = await consumeBody(this);
    const mediaType = contentType.split(";", 1)[0].trim().toLowerCase();
    if (mediaType === "application/x-www-form-urlencoded") return parseUrlEncoded(new TextDecoder().decode(data));
    if (mediaType === "multipart/form-data") return parseMultipart(data, contentType);
    throw new TypeError("Request body is not form data");
  }
  get [Symbol.toStringTag]() { return "Body"; }
}

async function consumeBody(body) {
  if (body.__bodyConsumed || body.__bodyStream?.locked) throw new TypeError("Body has already been used");
  if (body.__bodyStream === null) return new Uint8Array();
  body.__bodyConsumed = true;
  return consumeStream(body.__bodyStream);
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

function defaultSignal() {
  return typeof AbortController === "function" ? new AbortController().signal : { aborted: false };
}

function requestSignal(value) {
  if (typeof AbortSignal === "function" && value instanceof AbortSignal) return value;
  throw new TypeError("Invalid signal");
}

function requestBody(value) {
  if (value instanceof ReadableStream) return { body: null, stream: value, contentType: null };
  if (value instanceof FormData) {
    const serialized = formDataBody(value);
    return { body: serialized.body, stream: null, contentType: serialized.contentType };
  }
  return { body: bytes(value), stream: null, contentType: bodyInitType(value) };
}

function requestInit(request, source) {
  return {
    method: request.method,
    headers: request.headers,
    body: request.__stream ?? request.__body?.slice(),
    cache: request.cache,
    credentials: request.credentials,
    destination: request.destination,
    integrity: request.integrity,
    keepalive: request.keepalive,
    mode: request.mode,
    redirect: request.redirect,
    referrer: request.referrer,
    referrerPolicy: request.referrerPolicy,
    cf: request.cf,
    fetcher: request.fetcher,
    duplex: request.duplex,
    ...(source ? { body: source } : {}),
  };
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
  constructor(input, init = {}) {
    super();
    markHostObject(this, "Request");
    init = init ?? {};
    const source = input instanceof Request ? input : null;
    const hasBody = init.body !== undefined;
    if (source && !hasBody && (source.bodyUsed || source.body?.locked)) throw new TypeError("Cannot construct a Request from a used body");
    const method = requestMethod(init.method === undefined ? source?.method ?? "GET" : init.method);
    let inputBody = hasBody ? init.body : source?.__body;
    let inputStream = null;
    if (!hasBody && source?.body != null) {
      inputBody = source.__body?.slice() ?? null;
      inputStream = transferBody(source.body);
      source.__bodyConsumed = true;
    }
    const serialized = inputBody instanceof FormData ? requestBody(inputBody) : null;
    if ((method === "GET" || method === "HEAD") && (inputBody != null || inputStream !== null)) throw new TypeError("Request with GET/HEAD method cannot have body");
    const body = inputStream === null && inputBody != null ? requestBody(serialized?.body ?? inputBody) : { body: inputBody ?? null, stream: inputStream, contentType: null };
    const url = source?.url ?? new URL(input).href;
    const headers = new Headers(init.headers !== undefined ? init.headers : source?.headers);
    if ((inputBody != null || inputStream !== null) && !headers.has("content-type")) {
      const type = serialized?.contentType ?? body.contentType;
      if (type) headers.set("content-type", type);
    }
    hidden(this, "__url", url);
    hidden(this, "__method", method);
    hidden(this, "__headers", headers);
    hidden(this, "__body", body.body);
    hidden(this, "__stream", body.stream);
    hidden(this, "__bodyStream", this.__stream ?? (this.__body == null ? null : bodyStream(this.__body)));
    hidden(this, "__bodyConsumed", false);
    hidden(this, "__bodyDisturbed", false);
    hidden(this, "__cache", init.cache ?? source?.cache);
    hidden(this, "__cf", init.cf ?? source?.cf);
    hidden(this, "__fetcher", init.fetcher ?? source?.fetcher);
    if (init.integrity === null) throw new TypeError("Invalid integrity");
    hidden(this, "__integrity", init.integrity === undefined ? source?.integrity ?? "" : init.integrity);
    hidden(this, "__keepalive", Boolean(init.keepalive ?? source?.keepalive ?? false));
    hidden(this, "__redirect", requestRedirect(init.redirect === undefined ? source?.redirect ?? "follow" : init.redirect));
    const inheritedSignal = init.signal === undefined ? source?.signal : init.signal;
    hidden(this, "__signal", inheritedSignal == null ? defaultSignal() : requestSignal(inheritedSignal));
    hidden(this, "__signalProvided", init.signal === undefined ? source?.__signalProvided ?? false : init.signal != null);
  }

  get cache() { return this.__cache; }
  get cf() { return this.__cf; }
  get fetcher() { return this.__fetcher; }
  get headers() { return this.__headers; }
  get integrity() { return this.__integrity; }
  get keepalive() { return this.__keepalive; }
  get method() { return this.__method; }
  get redirect() { return this.__redirect; }
  get signal() { return this.__signal; }
  get url() { return this.__url; }
  get [Symbol.toStringTag]() { return "Request"; }

  clone() {
    if (this.bodyUsed || this.body?.locked) throw new TypeError("Body has already been used");
    if (this.__stream !== null) {
      const [first, second] = this.__stream.tee();
      this.__stream = first;
      this.__bodyStream = first;
      return new Request(this.url, requestInit(this, second));
    }
    return new Request(this.url, requestInit(this));
  }
}

export class Response extends Body {
  constructor(body = null, init = {}) {
    super();
    markHostObject(this, "Response");
    init = init ?? {};
    const serialized = body instanceof FormData ? requestBody(body) : null;
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
    const inputEncoding = init instanceof Response ? init.__encodeBody : init.encodeBody;
    const encodeBody = inputEncoding === undefined ? "automatic" : string(inputEncoding);
    if (encodeBody !== "automatic" && encodeBody !== "manual") throw new TypeError(`encodeBody: unexpected value: ${encodeBody}`);
    const headers = new Headers(init.headers);
    if (body != null && !(body instanceof ReadableStream) && !headers.has("content-type")) {
      const type = serialized?.contentType ?? bodyInitType(body);
      if (type) headers.set("content-type", type);
    }
    const stream = body instanceof ReadableStream ? body : null;
    const bodyBytes = stream === null && body != null ? bytes(serialized?.body ?? body) : null;
    hidden(this, "__status", status);
    hidden(this, "__statusText", statusText);
    hidden(this, "__encodeBody", encodeBody);
    hidden(this, "__headers", headers);
    hidden(this, "__stream", stream);
    hidden(this, "__body", bodyBytes);
    hidden(this, "__bodyStream", stream ?? (body == null ? null : bodyStream(bodyBytes)));
    hidden(this, "__tokamak_body", bodyBytes);
    hidden(this, "__ok", status >= 200 && status < 300);
    hidden(this, "__redirected", Boolean(init.redirected));
    hidden(this, "__type", init.type ?? "default");
    hidden(this, "__url", string(init.url ?? ""));
    hidden(this, "__cf", init.cf);
    hidden(this, "__webSocket", webSocket);
    hidden(this, "__bodyConsumed", false);
    hidden(this, "__bodyDisturbed", false);
    hidden(this, "__error", false);
  }

  get cf() { return this.__cf; }
  get headers() { return this.__headers; }
  get ok() { return this.__ok; }
  get redirected() { return this.__redirected; }
  get status() { return this.__status; }
  get statusText() { return this.__statusText; }
  get type() { return this.__type; }
  get url() { return this.__url; }
  get webSocket() { return this.__webSocket; }
  get [Symbol.toStringTag]() { return "Response"; }

  clone() {
    if (this.__error) return Response.error();
    if (this.__bodyConsumed || this.body?.locked) throw new TypeError("Body has already been used");
    if (isDisturbed(this.__bodyStream)) {
      const [first, second] = this.__bodyStream.tee();
      this.__bodyStream = first;
      this.__stream = this.__stream === null ? null : first;
      this.__bodyDisturbed = true;
      return new Response(second, responseInit(this));
    }
    if (this.__stream !== null) {
      const [first, second] = this.__stream.tee();
      this.__stream = first;
      this.__bodyStream = first;
      return new Response(second, responseInit(this));
    }
    return new Response(this.__body?.slice() ?? null, responseInit(this));
  }

  static error() {
    const response = new Response(null);
    response.__status = 0;
    response.__statusText = "";
    response.__ok = false;
    response.__type = "error";
    response.__error = true;
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
}

function responseInit(response) {
  return {
    status: response.status,
    statusText: response.statusText,
    headers: response.headers,
    url: response.url,
    redirected: response.redirected,
    type: response.type,
    cf: response.cf,
    webSocket: response.webSocket,
  };
}

function relativeIndex(value, length) {
  const number = Number(value);
  if (Number.isNaN(number)) return 0;
  if (number === -Infinity) return 0;
  if (number === Infinity) return length;
  return number < 0 ? Math.max(length + Math.trunc(number), 0) : Math.min(Math.trunc(number), length);
}

function blobPartBytes(value) {
  if (value instanceof Blob) return value.__bytes.slice();
  if (typeof value === "string") return new TextEncoder().encode(value);
  if (value instanceof ArrayBuffer) return new Uint8Array(value).slice();
  if (ArrayBuffer.isView(value)) return new Uint8Array(value.buffer, value.byteOffset, value.byteLength).slice();
  return new TextEncoder().encode(usvString(value));
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
  const boundary = "----tokamak-" + crypto.randomUUID();
  const encoder = new TextEncoder();
  const chunks = [];
  for (const [name, value] of form) {
    const file = value instanceof File;
    chunks.push(encoder.encode("--" + boundary + "\r\nContent-Disposition: form-data; name=\"" + quoteHeader(name) + "\""
      + (file ? "; filename=\"" + quoteHeader(value.name) + "\"" : "")
      + (file ? "\r\nContent-Type: " + (value.type || "application/octet-stream") : "") + "\r\n\r\n"));
    chunks.push(file ? value.__bytes : encoder.encode(value), encoder.encode("\r\n"));
  }
  chunks.push(encoder.encode("--" + boundary + "--\r\n"));
  const length = chunks.reduce((total, value) => total + value.byteLength, 0);
  const body = new Uint8Array(length);
  let offset = 0;
  for (const value of chunks) {
    body.set(value, offset);
    offset += value.byteLength;
  }
  return { body, contentType: "multipart/form-data; boundary=" + boundary };
}

function quoteHeader(value) { return string(value).replace(/["\\\r\n]/g, character => "\\" + character); }

export class Cache {
  constructor(name = "default") { markHostObject(this); hidden(this, "__name", string(name)); }
  async match(request, options = {}) {
    if (!options.ignoreMethod && requestMethodForCache(request) !== "GET") return undefined;
    const entry = await cacheHost("cacheMatch", [cachePath(), this.__name, cacheKey(request, options)]);
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
      cachePath(), this.__name, cacheKey(request), {
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
    return cacheHost("cacheDelete", [cachePath(), this.__name, cacheKey(request, options)]);
  }
  async keys() { throw cacheNotImplemented("Cache", "keys"); }
  async add() { throw cacheNotImplemented("Cache", "add"); }
  async addAll() { throw cacheNotImplemented("Cache", "addAll"); }
}

export class CacheStorage {
  constructor() {
    markHostObject(this);
    this.default = new Cache();
    this.__caches = new Map([["default", this.default]]);
  }
  async open(name = "default") {
    const key = string(name);
    let cache = this.__caches.get(key);
    if (!cache) {
      cache = new Cache(key);
      this.__caches.set(key, cache);
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
function cachePath() { return String(globalThis.__tokamak_cache ?? ""); }
async function cacheHost(name, args) { return (await import("tokamak:host"))[name](...args); }
function cacheNotImplemented(type, method) { return new Error(`Failed to execute '${method}' on '${type}': the method is not implemented.`); }

export class EventSource extends EventTarget {
  constructor(url, options = {}) {
    super();
    options ??= {};
    hidden(this, "__url", options[eventSourceStream] ? "" : new URL(url, globalThis.__tokamak_request?.url).href);
    hidden(this, "__withCredentials", Boolean(options.withCredentials));
    hidden(this, "__readyState", EventSource.CONNECTING);
    hidden(this, "__onopen", null);
    hidden(this, "__onmessage", null);
    hidden(this, "__onerror", null);
    hidden(this, "__reader", null);
    hidden(this, "__controller", new AbortController());
    hidden(this, "__lastEventId", "");
    if (options[eventSourceStream]) {
      this.__readyState = EventSource.OPEN;
      void consumeEventSource(this, options[eventSourceStream], true);
    }
    else void connectEventSource(this, options);
  }

  get url() { return this.__url; }
  get readyState() { return this.__readyState; }
  get withCredentials() { return this.__withCredentials; }
  get onopen() { return this.__onopen; }
  set onopen(value) { this.__onopen = value === null ? null : value; }
  get onmessage() { return this.__onmessage; }
  set onmessage(value) { this.__onmessage = value === null ? null : value; }
  get onerror() { return this.__onerror; }
  set onerror(value) { this.__onerror = value === null ? null : value; }

  close() {
    if (this.__readyState === EventSource.CLOSED) return;
    this.__readyState = EventSource.CLOSED;
    this.__controller.abort();
    void this.__reader?.cancel().catch(() => {});
  }

  static from(stream) {
    if (!(stream instanceof ReadableStream)) throw new TypeError("EventSource.from requires a ReadableStream");
    return new EventSource("", { [eventSourceStream]: stream });
  }

}
EventSource.CONNECTING = 0;
EventSource.OPEN = 1;
EventSource.CLOSED = 2;
Object.assign(EventSource.prototype, {
  CONNECTING: EventSource.CONNECTING,
  OPEN: EventSource.OPEN,
  CLOSED: EventSource.CLOSED,
});

const eventSourceStream = Symbol("event-source-stream");

function emitEventSource(source, event) {
  source.dispatchEvent(event);
  const handler = source["on" + event.type];
  if (typeof handler === "function") handler.call(source, event);
}

function emitEventSourceError(source, error) {
  source.__readyState = EventSource.CONNECTING;
  emitEventSource(source, new ErrorEvent("error", { error, message: error?.message ?? String(error) }));
}

async function connectEventSource(source, options) {
  try {
    const fetcher = options.fetcher ?? globalThis;
    if (typeof fetcher.fetch !== "function") throw new TypeError("EventSource fetcher must provide fetch()");
    const response = await fetcher.fetch.call(fetcher, source.url, {
      headers: { accept: "text/event-stream" },
      signal: source.__controller.signal,
    });
    if (!response?.ok || response.body === null) throw new TypeError("EventSource response was not a stream");
    source.__readyState = EventSource.OPEN;
    emitEventSource(source, new Event("open"));
    await consumeEventSource(source, response.body, true);
  } catch (error) {
    if (source.__readyState !== EventSource.CLOSED) emitEventSourceError(source, error);
  }
}

async function consumeEventSource(source, stream, closeWhenDone) {
  const reader = stream.getReader();
  source.__reader = reader;
  const decoder = new TextDecoder();
  let input = "";
  let data = [];
  let eventName = "";
  const dispatch = () => {
    if (data.length === 0) return;
    emitEventSource(source, new MessageEvent(eventName || "message", {
      data: data.join("\n"),
      lastEventId: source.__lastEventId,
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
    else if (field === "id" && !fieldValue.includes("\0")) source.__lastEventId = fieldValue;
  };
  try {
    while (true) {
      const result = await reader.read();
      if (result.done) break;
      input += decoder.decode(result.value, { stream: true });
      let separator;
      while ((separator = input.indexOf("\n")) >= 0) {
        let value = input.slice(0, separator);
        if (value.endsWith("\r")) value = value.slice(0, -1);
        input = input.slice(separator + 1);
        line(value);
      }
    }
    input += decoder.decode();
    if (input !== "") line(input.endsWith("\r") ? input.slice(0, -1) : input);
    dispatch();
    if (closeWhenDone && source.__readyState !== EventSource.CLOSED) source.__readyState = EventSource.CLOSED;
  } catch (error) {
    if (source.__readyState !== EventSource.CLOSED) emitEventSourceError(source, error);
  } finally {
    if (source.__reader === reader) source.__reader = null;
  }
}

export { bodyStream, bytes };
