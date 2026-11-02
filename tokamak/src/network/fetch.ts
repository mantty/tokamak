import type { BodyReader, FetchedResponse, RequestDescriptor, ResponseParts } from "tokamak:host";
import { callFunction, isFunction, isIterable, isObject, markHostObject, objectOf, propertyOf } from "../globals/objects.ts";
import { cacheDelete, cacheMatch, cachePut, httpFetch, httpStatusText } from "tokamak:host";
import { randomUUID } from "../globals/crypto.ts";
import { hostObjectCopies, uncloneable } from "../globals/structured-clone.ts";
import { TextDecoder, TextEncoder } from "../streams/text.ts";
import { ReadableStream, drainedChunkBytes, isDisturbed, setStreamLength } from "../streams/web.ts";
import { ErrorEvent, Event, EventTarget, MessageEvent } from "../events/web.ts";
import { AbortController, AbortSignal } from "../events/abort.ts";
import { URL, URLSearchParams } from "./url.ts";
import { WebSocket, nativeWebSocket } from "./websocket.ts";
import { bufferSourceBytes, fieldTypeError, joinBytes, string, usvString } from "../globals/conversions.ts";

// A body's content: its bytes or stream, and the content type it implies.
interface Content {
  bytes: Uint8Array | null;
  stream: ReadableStream<unknown> | null;
  contentType: string | null;
}

function bytes(value: unknown): Uint8Array {
  if (value == null) return new Uint8Array();
  const copy = bufferSourceBytes(value)?.slice();
  if (copy) return copy;
  if (value instanceof Blob) return blobBytes(value).slice();
  if (value instanceof URLSearchParams) return new TextEncoder().encode(value.toString());
  if (value instanceof FormData) return formDataBody(value).bytes;
  return new TextEncoder().encode(string(value));
}

function bodyStream(value: unknown): ReadableStream<Uint8Array> {
  const data = value instanceof Uint8Array ? value.slice() : bytes(value);
  const stream = new ReadableStream<Uint8Array>({ start(controller) {
    if (data.byteLength > 0) controller.enqueue(data);
    controller.close();
  } });
  return setStreamLength(stream, data.byteLength);
}

function consumeStream(stream: ReadableStream<unknown>): Promise<Uint8Array<ArrayBuffer>> {
  const reader = stream.getReader();
  const chunks: Uint8Array[] = [];
  return (async () => {
    try {
      for (let result = await reader.read(); !result.done; result = await reader.read()) {
        const chunk = bufferSourceBytes(result.value);
        if (!chunk) throw new TypeError("This ReadableStream did not return bytes.");
        chunks.push(chunk.slice());
      }
    } finally {
      reader.releaseLock();
    }
    return joinBytes(chunks);
  })();
}

// Reads `stream` for the host: `read()` gives each chunk as bytes, then undefined.
function hostStreamReader(stream: ReadableStream<unknown>): BodyReader {
  const reader = stream.getReader();
  return {
    async read() {
      const { done, value } = await reader.read();
      return done ? undefined : drainedChunkBytes(value);
    },
    cancel: () => reader.cancel(),
  };
}

function bodyInitType(value: unknown): string | null {
  if (typeof value === "string") return "text/plain;charset=UTF-8";
  if (value instanceof URLSearchParams) return "application/x-www-form-urlencoded;charset=UTF-8";
  if (value instanceof Blob && value.type !== "") return value.type;
  return null;
}

// A body init's content, as a stream or bytes, and the content type it implies.
function bodyInit(value: unknown): Content {
  if (value instanceof ReadableStream) return { bytes: null, stream: value, contentType: null };
  if (value instanceof FormData) return formDataBody(value);
  return { bytes: bytes(value), stream: null, contentType: bodyInitType(value) };
}

function validateHeaderName(name: unknown): string {
  const value = string(name);
  if (!/^[!#$%&'*+.^_`|~0-9A-Za-z-]+$/.test(value)) throw new TypeError("Invalid header name");
  return value.toLowerCase();
}

function normalizeHeaderValue(value: unknown): string {
  const normalized = trimEnd(string(value).replace(/^[\t ]+/, ""), "\t ");
  if (/[\0-\x08\x0a-\x1f\x7f]/.test(normalized)) throw new TypeError("Invalid header value");
  return normalized;
}

// `value` without trailing `characters`. A loop, since a regular expression
// anchored only at the end takes quadratic time on a long run of them.
function trimEnd(value: string, characters: string): string {
  let end = value.length;
  while (end > 0 && characters.includes(value[end - 1])) end -= 1;
  return value.slice(0, end);
}

export class Headers {
  #values = new Map<string, string[]>();

  constructor(init?: unknown) {
    markHostObject(this, "Headers");
    if (isObject(init) && #values in init) {
      for (const [name, values] of init.#values) this.#values.set(name, values.slice());
    } else if (isIterable(init)) {
      for (const entry of init) {
        if (!isIterable(entry)) throw new TypeError("Invalid header pair");
        const pair = [...entry];
        if (pair.length !== 2) throw new TypeError("Invalid header pair");
        this.append(pair[0], pair[1]);
      }
    } else if (init != null) {
      const record = objectOf(init);
      for (const name of Object.keys(record)) this.append(name, record[name]);
    }
  }

  append(name: unknown, value: unknown): void {
    const key = validateHeaderName(name);
    const next = normalizeHeaderValue(value);
    const values = this.#values.get(key) ?? [];
    if (key === "set-cookie" || values.length === 0) values.push(next);
    else values[0] += ", " + next;
    this.#values.set(key, values);
  }
  set(name: unknown, value: unknown): void { this.#values.set(validateHeaderName(name), [normalizeHeaderValue(value)]); }
  get(name: unknown): string | null { return this.#values.get(validateHeaderName(name))?.join(", ") ?? null; }
  getAll(name: unknown): string[] { return this.#values.get(validateHeaderName(name))?.slice() ?? []; }
  has(name: unknown): boolean { return this.#values.has(validateHeaderName(name)); }
  delete(name: unknown): void { this.#values.delete(validateHeaderName(name)); }
  getSetCookie(): string[] { return this.#values.get("set-cookie")?.slice() ?? []; }
  entries() { return headerEntries(this.#values).values(); }
  keys() { return headerEntries(this.#values).map(entry => entry[0]).values(); }
  values() { return headerEntries(this.#values).map(entry => entry[1]).values(); }
  forEach(callback: unknown, thisArg?: unknown): void { for (const [name, value] of this) callFunction(callback, thisArg, value, name, this); }
  [Symbol.iterator]() { return this.entries(); }
  get [Symbol.toStringTag]() { return "Headers"; }

  static {
    hostObjectCopies.set("Headers", headers => new Headers(headers));
  }
}

function headerEntries(values: Map<string, string[]>): [string, string][] {
  const entries: [string, string][] = [];
  for (const [name, list] of [...values].sort(([left], [right]) => left < right ? -1 : left > right ? 1 : 0)) {
    if (name === "set-cookie") for (const value of list) entries.push([name, value]);
    else entries.push([name, list.join(", ")]);
  }
  return entries;
}

// The bytes a Blob holds.
let blobBytes: (blob: Blob) => Uint8Array;
// The type a Blob holds.
let blobType: (blob: Blob) => string;
// Sets a Blob's type as given.
let setBlobType: (blob: Blob, type: string) => void;

/** A Blob of `bytes` whose type is `type` unchanged, as workerd's own Blobs are. */
export function exactBlob(bytes: Uint8Array, type: string): Blob {
  const blob = new Blob([bytes]);
  setBlobType(blob, type);
  return blob;
}

export class Blob {
  #bytes: Uint8Array;
  #type: string;

  constructor(parts: unknown = [], options: unknown = {}) {
    markHostObject(this, "Blob");
    if (!isIterable(parts)) throw new TypeError("Blob parts must be iterable");
    const chunks = [...parts].map(blobPartBytes);
    this.#bytes = joinBytes(chunks);
    this.#type = mimeType(propertyOf(options, "type"));
  }
  get size(): number { return this.#bytes.byteLength; }
  get type(): string { return this.#type; }
  async text(): Promise<string> { return new TextDecoder().decode(this.#bytes); }
  async arrayBuffer(): Promise<ArrayBuffer> { return this.#bytes.slice().buffer; }
  async bytes(): Promise<Uint8Array> { return this.#bytes.slice(); }
  stream(): ReadableStream<Uint8Array> { return bodyStream(this.#bytes); }
  slice(start: unknown = 0, end: unknown = this.size, contentType: unknown = ""): Blob {
    const first = relativeIndex(start, this.size);
    const last = relativeIndex(end, this.size);
    return new Blob([first < last ? this.#bytes.subarray(first, last) : new Uint8Array()], { type: contentType });
  }
  get [Symbol.toStringTag]() { return "Blob"; }

  static {
    blobBytes = blob => blob.#bytes;
    blobType = blob => blob.#type;
    setBlobType = (blob, type) => { blob.#type = type; };
    hostObjectCopies.set("Blob", blob => {
      if (!(#bytes in blob)) throw uncloneable();
      return new Blob([blob.#bytes], { type: blob.#type });
    });
  }
}

export class File extends Blob {
  #name: string;
  #lastModified: number;

  constructor(parts: unknown, name: unknown, options: unknown = {}) {
    if (arguments.length < 2) throw new TypeError("File name is required");
    super(parts, options);
    markHostObject(this, "File");
    this.#name = usvString(name);
    const lastModified = propertyOf(options, "lastModified");
    const modified = Number(lastModified === undefined ? Date.now() : lastModified);
    this.#lastModified = Number.isNaN(modified) ? 0 : modified;
  }
  get name(): string { return this.#name; }
  get lastModified(): number { return this.#lastModified; }
  get [Symbol.toStringTag]() { return "File"; }
}

function formDataValue(value: unknown, filename: unknown): string | File {
  if (value instanceof File && filename === undefined) return value;
  if (value instanceof Blob) {
    const name = filename === undefined ? (value instanceof File ? value.name : "blob") : usvString(filename);
    const file = new File([value], name);
    setBlobType(file, blobType(value));
    return file;
  }
  return usvString(value);
}

// A form entry's name and value.
type FormEntry = [string, string | File];

export class FormData {
  #entries: FormEntry[] = [];

  constructor() { markHostObject(this); }
  append(name: unknown, value: unknown, filename?: unknown): void { this.#entries.push([usvString(name), formDataValue(value, filename)]); }
  set(name: unknown, value: unknown, filename?: unknown): void {
    const key = usvString(name);
    const item = formDataValue(value, filename);
    const index = this.#entries.findIndex(([entryName]) => entryName === key);
    if (index < 0) this.#entries.push([key, item]);
    else {
      this.#entries[index] = [key, item];
      this.#entries = this.#entries.filter(([entryName], entryIndex) => entryName !== key || entryIndex === index);
    }
  }
  get(name: unknown): string | File | null { return this.#entries.find(([key]) => key === usvString(name))?.[1] ?? null; }
  getAll(name: unknown): (string | File)[] { return this.#entries.filter(([key]) => key === usvString(name)).map(([, value]) => value); }
  has(name: unknown): boolean { return this.#entries.some(([key]) => key === usvString(name)); }
  delete(name: unknown): void { this.#entries = this.#entries.filter(([key]) => key !== usvString(name)); }
  entries() { return this.#entries.map(([key, value]): FormEntry => [key, value]).values(); }
  keys() { return this.#entries.map(([key]) => key).values(); }
  values() { return this.#entries.map(([, value]) => value).values(); }
  forEach(callback: unknown, thisArg?: unknown): void { for (const [key, value] of this) callFunction(callback, thisArg, value, key, this); }
  [Symbol.iterator]() { return this.entries(); }
  get [Symbol.toStringTag]() { return "FormData"; }
}

// The content to send, `{ bytes, stream }`, after which the body is used.
let takeBody: (target: Body) => Pick<Content, "bytes" | "stream">;
// The content for a copy of a body, teeing its stream so both read it.
let cloneBody: (target: Body) => Uint8Array | ReadableStream<unknown> | null;
// The stream a body exposes, or null.
let bodyOf: (target: Body) => ReadableStream<unknown> | null;

export abstract class Body {
  #bytes: Uint8Array | null;
  #stream: ReadableStream<unknown> | null;
  #body: ReadableStream<unknown> | null;
  #used = false;
  #disturbed = false;

  constructor(bytes: Uint8Array | null = null, stream: ReadableStream<unknown> | null = null) {
    if (new.target === Body) throw new TypeError("Illegal constructor");
    this.#bytes = bytes;
    this.#stream = stream;
    this.#body = stream ?? (bytes === null ? null : bodyStream(bytes));
  }

  abstract get headers(): Headers;
  get body(): ReadableStream<unknown> | null { return this.#body; }
  get bodyUsed(): boolean { return this.#used || this.#disturbed || isDisturbed(this.#body); }
  async arrayBuffer(): Promise<ArrayBuffer> { return (await this.#consume()).buffer; }
  async bytes(): Promise<Uint8Array> { return this.#consume(); }
  async blob(): Promise<Blob> {
    const data = await this.#consume();
    return exactBlob(data, extractMimeType(this.headers.get("content-type") ?? ""));
  }
  async text(): Promise<string> { return new TextDecoder().decode(await this.#consume()); }
  async json(): Promise<unknown> { return JSON.parse(await this.text()); }
  async formData(): Promise<FormData> {
    const contentType = this.headers.get("content-type") ?? "";
    const data = await this.#consume();
    const mediaType = contentType.split(";", 1)[0].trim().toLowerCase();
    if (mediaType === "application/x-www-form-urlencoded") return parseUrlEncoded(new TextDecoder().decode(data));
    if (mediaType === "multipart/form-data") return parseMultipart(data, contentType);
    throw new TypeError("Request body is not form data");
  }
  get [Symbol.toStringTag]() { return "Body"; }

  async #consume(): Promise<Uint8Array<ArrayBuffer>> {
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
      const body = target.#body;
      if (target.#used || body?.locked) throw new TypeError("Body has already been used");
      const disturbed = isDisturbed(body);
      if (body === null || (target.#stream === null && !disturbed)) return target.#bytes?.slice() ?? null;
      const [first, second] = body.tee();
      target.#body = first;
      if (target.#stream !== null) target.#stream = first;
      target.#disturbed ||= disturbed;
      return second;
    };
    bodyOf = target => target.#body;
  }
}

function requestMethod(value: unknown): string {
  if (value === null) throw new TypeError("Invalid request method");
  const method = string(value);
  if (!/^[!#$%&'*+.^_`|~0-9A-Za-z-]+$/.test(method)) throw new TypeError("Invalid request method");
  const upper = method.toUpperCase();
  return ["DELETE", "GET", "HEAD", "OPTIONS", "POST", "PUT"].includes(upper) ? upper : method;
}

function requestRedirect(value: unknown): string {
  if (value === null) throw new TypeError("Invalid redirect mode");
  const redirect = string(value).toLowerCase();
  if (redirect === "error") throw new TypeError('Invalid redirect value, must be one of "follow" or "manual" ("error" won\'t be implemented since it does not make sense at the edge; use "manual" and check the response status code).');
  if (!["follow", "manual"].includes(redirect)) throw new TypeError("Invalid redirect mode");
  return redirect;
}

function transferBody(stream: ReadableStream<unknown>): ReadableStream<Uint8Array> {
  const reader = stream.getReader();
  return new ReadableStream({
    type: "bytes",
    async pull(controller) {
      try {
        const { done, value } = await reader.read();
        if (done) { reader.releaseLock(); controller.close(); }
        else if (ArrayBuffer.isView(value)) controller.enqueue(value);
        else throw new TypeError("This ReadableStream did not return bytes.");
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
  #url: string;
  #method: string;
  #headers: Headers;
  #cache: unknown;
  #cf: unknown;
  #fetcher: unknown;
  #integrity: unknown;
  #keepalive: boolean;
  #redirect: string;
  #signal: AbortSignal;
  #signalProvided: boolean;

  constructor(input: unknown, init: unknown = {}) {
    const source = input instanceof Request ? input : null;
    const body = propertyOf(init, "body");
    const hasBody = body !== undefined;
    if (source && !hasBody && (source.bodyUsed || source.body?.locked)) throw new TypeError("Cannot construct a Request from a used body");
    const methodInit = propertyOf(init, "method");
    const method = requestMethod(methodInit === undefined ? source?.method ?? "GET" : methodInit);
    let content: Content = { bytes: null, stream: null, contentType: null };
    if (!hasBody && source?.body != null) {
      const taken = takeBody(source);
      if (taken.stream !== null) content = { bytes: taken.bytes?.slice() ?? null, stream: transferBody(taken.stream), contentType: null };
    }
    const inputBody = hasBody ? body : null;
    if ((method === "GET" || method === "HEAD") && (inputBody != null || content.stream !== null)) throw new TypeError("Request with GET/HEAD method cannot have body");
    if (inputBody != null) content = bodyInit(inputBody);
    const url = source?.url ?? new URL(input).href;
    const headersInit = propertyOf(init, "headers");
    const headers = new Headers(headersInit !== undefined ? headersInit : source?.headers);
    if (content.contentType && !headers.has("content-type")) headers.set("content-type", content.contentType);
    super(content.bytes, content.stream);
    markHostObject(this, "Request");
    this.#url = url;
    this.#method = method;
    this.#headers = headers;
    this.#cache = propertyOf(init, "cache") ?? source?.cache;
    this.#cf = propertyOf(init, "cf") ?? source?.cf;
    this.#fetcher = propertyOf(init, "fetcher") ?? source?.fetcher;
    const integrity = propertyOf(init, "integrity");
    if (integrity === null) throw new TypeError("Invalid integrity");
    this.#integrity = integrity === undefined ? source?.integrity ?? "" : integrity;
    this.#keepalive = Boolean(propertyOf(init, "keepalive") ?? source?.keepalive ?? false);
    const redirect = propertyOf(init, "redirect");
    this.#redirect = requestRedirect(redirect === undefined ? source?.redirect ?? "follow" : redirect);
    const signal = propertyOf(init, "signal");
    const inheritedSignal = signal === undefined ? source?.signal : signal;
    if (inheritedSignal != null && !(inheritedSignal instanceof AbortSignal)) throw new TypeError("Invalid signal");
    this.#signal = inheritedSignal ?? new AbortController().signal;
    this.#signalProvided = signal === undefined ? source !== null && source.#signalProvided : signal != null;
  }

  get cache(): unknown { return this.#cache; }
  get cf(): unknown { return this.#cf; }
  get fetcher(): unknown { return this.#fetcher; }
  get headers(): Headers { return this.#headers; }
  get integrity(): unknown { return this.#integrity; }
  get keepalive(): boolean { return this.#keepalive; }
  get method(): string { return this.#method; }
  get redirect(): string { return this.#redirect; }
  get signal(): AbortSignal { return this.#signal; }
  get url(): string { return this.#url; }
  get [Symbol.toStringTag]() { return "Request"; }

  clone(): Request {
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
      if (!(#url in request) || bodyOf(request) !== null || request.#signalProvided) throw uncloneable();
      const copy = keep(new Request(request.#url, { method: request.#method, headers: request.#headers, redirect: request.#redirect, cache: request.#cache }));
      copy.#cf = clone(request.#cf);
      return copy;
    });
  }
}

// The Request a Worker receives for the host's request descriptor and body bytes.
export function hostRequest(request: RequestDescriptor, body: Uint8Array | undefined): Request {
  return new Request(request.url, { method: request.method, headers: request.headers, body });
}

// The status line, headers, encoding, body (bytes or a host stream reader) and native WebSocket
// end the host sends for a Worker's response.
let hostResponse: (response: unknown) => ResponseParts;
// A fetch's response, with the URL it came from and whether a redirect led
// there, which only fetch sets: a Response's init cannot.
let fetchedResponse: (body: ReadableStream<Uint8Array> | null, init: object, url: string, redirected: boolean) => Response;

export class Response extends Body {
  #status: number;
  #statusText: string;
  #encodeBody: string;
  #headers: Headers;
  #redirected = false;
  #url = "";
  #cf: unknown;
  #webSocket: WebSocket | null;

  constructor(body: unknown = null, init: unknown = {}) {
    const inputStatus = propertyOf(init, "status");
    const status = Math.trunc(Number(inputStatus === undefined ? 200 : inputStatus));
    const webSocket = propertyOf(init, "webSocket") ?? null;
    if (webSocket !== null && !(webSocket instanceof WebSocket)) throw fieldTypeError("webSocket", "ResponseInit", "WebSocket");
    if (webSocket !== null) {
      if (status !== 101) throw new RangeError("Responses with a WebSocket must have status code 101.");
    } else if (!Number.isInteger(status) || status < 200 || status > 599) {
      throw new RangeError("Invalid response status code");
    }
    if ([204, 205, 304].includes(status) && body !== null && body !== undefined) throw new TypeError("Response with null body status cannot have a body");
    const inputStatusText = propertyOf(init, "statusText");
    const statusText = string(inputStatusText === undefined ? httpStatusText(status) : inputStatusText);
    if (/[\0-\x1f\x7f]/.test(statusText)) throw new TypeError("Invalid response status text");
    const inputEncoding = init instanceof Response ? init.#encodeBody : propertyOf(init, "encodeBody");
    const encodeBody = inputEncoding === undefined ? "automatic" : string(inputEncoding);
    if (encodeBody !== "automatic" && encodeBody !== "manual") throw new TypeError(`encodeBody: unexpected value: ${encodeBody}`);
    const headers = new Headers(propertyOf(init, "headers"));
    const content: Content = body == null ? { bytes: null, stream: null, contentType: null } : bodyInit(body);
    if (content.contentType && !headers.has("content-type")) headers.set("content-type", content.contentType);
    super(content.bytes, content.stream);
    markHostObject(this, "Response");
    this.#status = status;
    this.#statusText = statusText;
    this.#encodeBody = encodeBody;
    this.#headers = headers;
    this.#cf = propertyOf(init, "cf");
    this.#webSocket = webSocket;
  }

  get cf(): unknown { return this.#cf; }
  get headers(): Headers { return this.#headers; }
  get ok(): boolean { return this.#status >= 200 && this.#status < 300; }
  get redirected(): boolean { return this.#redirected; }
  get status(): number { return this.#status; }
  get statusText(): string { return this.#statusText; }
  // Only Response.error() has status 0; workerd has no other response types.
  get type(): string { return this.#status === 0 ? "error" : "default"; }
  get url(): string { return this.#url; }
  get webSocket(): WebSocket | null { return this.#webSocket; }
  get [Symbol.toStringTag]() { return "Response"; }

  clone(): Response {
    if (this.#status === 0) return Response.error();
    const clone = new Response(cloneBody(this), {
      status: this.#status,
      statusText: this.#statusText,
      headers: this.#headers,
      cf: this.#cf,
      webSocket: this.#webSocket,
    });
    clone.#url = this.#url;
    clone.#redirected = this.#redirected;
    return clone;
  }

  static error(): Response {
    const response = new Response(null);
    response.#status = 0;
    response.#statusText = "";
    return response;
  }
  static json(data: unknown, init: unknown = {}): Response {
    const options = objectOf(init ?? {});
    const headers = new Headers(options.headers);
    if (!headers.has("content-type")) headers.set("content-type", "application/json");
    // JSON.stringify gives undefined for undefined, functions and symbols.
    const body: string | undefined = JSON.stringify(data);
    return new Response(body === undefined ? "undefined" : body, { ...options, headers });
  }
  static redirect(url: unknown, status: unknown = 302): Response {
    const code = Math.trunc(Number(status));
    if (![301, 302, 303, 307, 308].includes(code)) throw new RangeError("Invalid redirect status code");
    return new Response(null, { status: code, headers: { location: new URL(url).href } });
  }

  static {
    hostObjectCopies.set("Response", (response, clone, keep) => {
      if (!(#status in response) || bodyOf(response) !== null || response.#webSocket !== null) throw uncloneable();
      const copy = keep(response.#status === 0 ? Response.error() : new Response(null, { status: response.#status, statusText: response.#statusText, headers: response.#headers }));
      copy.#cf = clone(response.#cf);
      return copy;
    });
    hostResponse = response => {
      if (!isObject(response) || !(#status in response)) throw new TypeError("Incorrect type for Promise: the Promise did not resolve to 'Response'.");
      const { bytes, stream } = takeBody(response);
      return {
        status: response.#status,
        statusText: response.#statusText,
        headers: [...response.#headers],
        encodeBody: response.#encodeBody,
        body: bytes ?? (stream === null ? new Uint8Array() : hostStreamReader(stream)),
        webSocket: response.#webSocket === null ? undefined : nativeWebSocket(response.#webSocket),
      };
    };
    fetchedResponse = (body, init, url, redirected) => {
      const response = new Response(body, init);
      response.#url = url;
      response.#redirected = redirected;
      return response;
    };
  }
}

function relativeIndex(value: unknown, length: number): number {
  const number = Number(value);
  if (Number.isNaN(number) || number === -Infinity) return 0;
  if (number === Infinity) return length;
  return number < 0 ? Math.max(length + Math.trunc(number), 0) : Math.min(Math.trunc(number), length);
}

function blobPartBytes(value: unknown): Uint8Array {
  if (value instanceof Blob) return blobBytes(value).slice();
  return bufferSourceBytes(value)?.slice() ?? new TextEncoder().encode(string(value));
}

function mimeType(value: unknown): string {
  const type = string(value === undefined ? "" : value);
  return /^[\x20-\x7e]*$/.test(type) ? type.toLowerCase() : "";
}

const HTTP_WHITESPACE = "\t\n\r ";
// workerd's token characters, which include a backslash.
const MIME_TOKEN = /^[!#$%&'*+\\\-.^_`|~0-9A-Za-z]+$/;
// workerd checks a parameter value's UTF-8 bytes, which every non-ASCII
// character passes.
const MIME_PARAMETER_VALUE = /^[^\x00-\x08\x0a-\x1f\x7f]*$/;

// A MIME type's essence, `type/subtype`, and its parameters.
interface MimeType {
  essence: string;
  parameters: Map<string, string>;
}

// The MIME type in a Content-Type value, serialized, as workerd's
// `MimeType::extract` and `toString` give it; "" when there is none.
function extractMimeType(value: string): string {
  let found: MimeType | null = null;
  for (let start = 0; start < value.length;) {
    const separator = mimeListSeparator(value, start);
    const parsed = parseMimeType(value.slice(start, separator));
    if (parsed && parsed.essence !== "*/*") {
      const charset = found?.essence === parsed.essence ? found.parameters.get("charset") : undefined;
      if (charset !== undefined && !parsed.parameters.has("charset")) parsed.parameters.set("charset", charset);
      found = parsed;
    }
    start = separator + 1;
  }
  if (found === null) return "";
  let type = found.essence;
  for (const [name, parameter] of found.parameters) {
    const quoted = parameter === "" || !MIME_TOKEN.test(parameter);
    type += (name === "boundary" ? "; " : ";") + name + "="
      + (quoted ? '"' + parameter.replace(/["\\]/g, "\\$&") + '"' : parameter);
  }
  return type;
}

// The index of the first comma in `value` from `start` outside quotes and not
// after a backslash, or the length of `value`.
function mimeListSeparator(value: string, start: number): number {
  for (let index = start; index < value.length; index += 1) {
    if (value[index] === '"' && value[index - 1] !== "\\") {
      index += 1;
      while (index < value.length && (value[index] !== '"' || value[index - 1] === "\\")) index += 1;
    } else if (value[index] === "," && value[index - 1] !== "\\") return index;
  }
  return value.length;
}

// `{ essence, parameters }` for one MIME type, as workerd's
// `MimeType::tryParse` reads it, or null. The parsers read by index: slicing
// off the rest of the value at each parameter would take quadratic time.
function parseMimeType(value: string): MimeType | null {
  const start = skipWhitespace(value, 0);
  const slash = value.indexOf("/", start);
  const type = value.slice(start, slash);
  if (slash < 0 || !MIME_TOKEN.test(type)) return null;
  const semicolon = indexOrEnd(value, ";", slash + 1);
  const subtype = trimEnd(value.slice(slash + 1, semicolon), HTTP_WHITESPACE);
  if (!MIME_TOKEN.test(subtype)) return null;
  const parameters = new Map<string, string>();
  for (let index = semicolon + 1; index < value.length;) index = readMimeParameter(value, index, parameters);
  return { essence: type.toLowerCase() + "/" + subtype.toLowerCase(), parameters };
}

// Adds the parameter at `start` in `value` to `parameters` unless workerd
// ignores it, and returns the index after it.
function readMimeParameter(value: string, start: number, parameters: Map<string, string>): number {
  const nameStart = skipWhitespace(value, start);
  let equals = nameStart;
  while (equals < value.length && value[equals] !== ";" && value[equals] !== "=") equals += 1;
  if (value[equals] !== "=") return equals + 1;
  const name = value.slice(nameStart, equals);
  let parameter: string;
  let end: number;
  if (MIME_TOKEN.test(name) && value[equals + 1] === '"') {
    [parameter, end] = mimeQuotedString(value, equals + 1);
    end = indexOrEnd(value, ";", end);
  } else {
    end = indexOrEnd(value, ";", equals + 1);
    parameter = trimEnd(value.slice(equals + 1, end), HTTP_WHITESPACE);
    if (parameter === "") return end + 1;
  }
  const key = name.toLowerCase();
  if (MIME_TOKEN.test(name) && MIME_PARAMETER_VALUE.test(parameter) && !parameters.has(key)) parameters.set(key, parameter);
  return end + 1;
}

// The value of the quoted string at `start` in `value`, and the index after it.
function mimeQuotedString(value: string, start: number): [string, number] {
  let result = "";
  let index = start + 1;
  while (index < value.length) {
    const character = value[index++];
    if (character === '"') break;
    result += character === "\\" && index < value.length ? value[index++] : character;
  }
  return [result, index];
}

// The index of the first character from `start` in `value` that is not HTTP
// whitespace.
function skipWhitespace(value: string, start: number): number {
  let index = start;
  while (index < value.length && HTTP_WHITESPACE.includes(value[index])) index += 1;
  return index;
}

// The index of `search` in `value` from `start`, or the length of `value`.
function indexOrEnd(value: string, search: string, start: number): number {
  const index = value.indexOf(search, start);
  return index < 0 ? value.length : index;
}

function parseUrlEncoded(value: string): FormData {
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

// `value` with each `+` a space and each `%XX` escape a byte; a `%` that
// starts no escape is dropped.
function decodeFormComponent(value: string): string {
  const encoder = new TextEncoder();
  const chunks: Uint8Array[] = [];
  for (const [, escape, text] of value.replace(/\+/g, " ").matchAll(/%([0-9A-Fa-f]{2})|([^%]+)/g)) {
    chunks.push(text === undefined ? Uint8Array.of(Number.parseInt(escape, 16)) : encoder.encode(text));
  }
  return new TextDecoder().decode(joinBytes(chunks));
}

// The entries of a multipart/form-data body, read as workerd's
// `FormData::parseFormDataImpl` reads them: text before the first boundary is
// skipped, and line breaks are CRLF or LF.
function parseMultipart(data: Uint8Array, contentType: string): FormData {
  const delimiter = new TextEncoder().encode("\n--" + multipartBoundary(contentType));
  const first = indexOfBytes(data, delimiter.subarray(1));
  let position = first < 0 ? data.length : first + delimiter.length - 1;
  if (position >= data.length) throw new TypeError("Multipart body has no initial boundary");
  const form = new FormData();
  while (data[position] !== 45 || data[position + 1] !== 45) {
    position = appendMultipartPart(form, data, afterLineBreak(data, position), delimiter);
  }
  return form;
}

// The index after the line break at `position`, which follows a boundary.
function afterLineBreak(data: Uint8Array, position: number): number {
  if (data[position] === 10) return position + 1;
  if (data[position] === 13 && data[position + 1] === 10) return position + 2;
  throw new TypeError("Multipart boundary is not followed by a line break or --");
}

// Appends the part at `start` to `form`, and returns the index after the
// delimiter that ends it.
function appendMultipartPart(form: FormData, data: Uint8Array, start: number, delimiter: Uint8Array): number {
  const headerEnd = multipartHeaderEnd(data, start);
  if (headerEnd < 0) throw new TypeError("Multipart headers have no end");
  const lastLineBreak = data[headerEnd - 2] === 13 ? 2 : 1;
  const headerText = new TextDecoder("utf-8", { ignoreBOM: true }).decode(data.subarray(start, headerEnd - lastLineBreak));
  const headers = parseMultipartHeaders(headerText);
  const disposition = multipartDisposition(headers.get("content-disposition"));
  const end = indexOfBytes(data, delimiter, headerEnd);
  if (end < 0 || end + delimiter.length >= data.length) throw new TypeError("Multipart part has no closing boundary");
  const value = data.subarray(headerEnd, data[end - 1] === 13 ? end - 1 : end);
  if (disposition.filename !== undefined) {
    const file = new File([value], disposition.filename);
    setBlobType(file, headers.get("content-type") ?? "");
    form.append(disposition.name, file);
  } else form.append(disposition.name, new TextDecoder().decode(value));
  return end + delimiter.length;
}

// The index after the first match of /\r?\n\r?\n/ from `start`, which ends a
// part's headers, or -1.
function multipartHeaderEnd(data: Uint8Array, start: number): number {
  for (let index = data.indexOf(10, start); index >= 0; index = data.indexOf(10, index + 1)) {
    if (data[index + 1] === 10) return index + 2;
    if (data[index + 1] === 13 && data[index + 2] === 10) return index + 3;
  }
  return -1;
}

function multipartBoundary(contentType: string): string {
  const boundary = parseMimeType(contentType)?.parameters.get("boundary");
  if (boundary === undefined) throw new TypeError("Multipart boundary is missing");
  return boundary;
}

// A header line as kj's `HttpHeaders::tryParse` reads it: a token, a colon,
// the value without leading tabs and spaces, and a line break that no folded
// line follows.
const MULTIPART_HEADER = /([^\0-\x20\x7f()<>@,;:\\"/[\]?={}]+):[\t ]*((?:[^\0\r\n\t ][^\0\r\n]*)?)(?:\r\n|\r(?!\n)|\n|$)(?![\t ])/y;

// A part's headers by lowercase name, with repeated headers joined by commas.
function parseMultipartHeaders(text: string): Map<string, string> {
  const headers = new Map<string, string>();
  for (MULTIPART_HEADER.lastIndex = 0; MULTIPART_HEADER.lastIndex < text.length;) {
    const match = MULTIPART_HEADER.exec(text);
    if (!match) throw new TypeError("Invalid multipart headers");
    const name = match[1].toLowerCase();
    const previous = headers.get(name);
    headers.set(name, previous === undefined ? match[2] : `${previous}, ${match[2]}`);
  }
  return headers;
}

// workerd's Content-Disposition grammar: a type, then parameters with
// non-empty quoted values, read until one does not match.
const DISPOSITION_TYPE = /^[\t\n\v\f\r ]*([\w-]+)[\t\n\v\f\r ]*/;
const DISPOSITION_PARAMETERS = /;[\t\n\v\f\r ]*([\w-]+)[\t\n\v\f\r ]*=[\t\n\v\f\r ]*"((?:[^"\n\\]|\\[^\n])+)"[\t\n\v\f\r ]*/gy;

// The last name and filename a form-data part's disposition gives.
function multipartDisposition(value: string | undefined): { name: string; filename: string | undefined } {
  const input = value ?? "";
  const type = DISPOSITION_TYPE.exec(input);
  if (type?.[1] !== "form-data") throw new TypeError("Invalid multipart disposition");
  let name: string | undefined;
  let filename: string | undefined;
  for (const [, key, quoted] of input.slice(type[0].length).matchAll(DISPOSITION_PARAMETERS)) {
    const parameter = quoted.replace(/\\([^\n])/g, "$1");
    if (key === "name") name = parameter;
    else if (key === "filename") filename = parameter;
  }
  if (name === undefined) throw new TypeError("Invalid multipart disposition");
  return { name, filename };
}

function matchesBytes(haystack: Uint8Array, needle: Uint8Array, start: number): boolean {
  if (start < 0 || start + needle.length > haystack.length) return false;
  for (let offset = 0; offset < needle.length; offset += 1) if (haystack[start + offset] !== needle[offset]) return false;
  return true;
}

function indexOfBytes(haystack: Uint8Array, needle: Uint8Array, start = 0): number {
  for (let index = start; index <= haystack.length - needle.length; index += 1) {
    if (matchesBytes(haystack, needle, index)) return index;
  }
  return -1;
}

function formDataBody(form: FormData) {
  const boundary = "----tokamak-" + randomUUID();
  const encoder = new TextEncoder();
  const chunks: Uint8Array[] = [];
  for (const [name, value] of form) {
    const file = value instanceof File;
    chunks.push(encoder.encode("--" + boundary + "\r\nContent-Disposition: form-data; name=\"" + quoteHeader(name) + "\""
      + (file ? "; filename=\"" + quoteHeader(value.name) + "\"" : "")
      + (file ? "\r\nContent-Type: " + (value.type || "application/octet-stream") : "") + "\r\n\r\n"));
    chunks.push(file ? blobBytes(value) : encoder.encode(value), encoder.encode("\r\n"));
  }
  chunks.push(encoder.encode("--" + boundary + "--\r\n"));
  const body = joinBytes(chunks);
  return { bytes: body, stream: null, contentType: "multipart/form-data; boundary=" + boundary };
}

function quoteHeader(value: unknown): string { return string(value).replace(/["\\\r\n]/g, character => "\\" + character); }


export async function fetch(input: unknown, init?: unknown): Promise<Response> {
  const request = new Request(input, init);
  if (request.signal.aborted) throw request.signal.reason;
  const { bytes, stream } = takeBody(request);
  const { fetch: task, upload, response: pending } = httpFetch({
    url: request.url,
    method: request.method,
    headers: [...request.headers],
    body: bytes ?? stream ?? undefined,
    redirect: request.redirect,
  });
  globalThis.__tokamak_context.waitUntil(upload);
  const abort = () => { task.abort(); };
  request.signal.addEventListener("abort", abort, { once: true });
  const finish = () => { request.signal.removeEventListener("abort", abort); };
  let response: FetchedResponse;
  try {
    response = await pending;
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
        const chunk = await task.read();
        request.signal.throwIfAborted();
        if (chunk === undefined) { finish(); controller.close(); }
        else controller.enqueue(chunk);
      } catch (error) {
        task.cancel(); finish();
        controller.error(request.signal.aborted ? fetchAbortReason(request.signal) : error);
      }
    },
    cancel() { task.cancel(); finish(); },
  });
  if (response.bodyless) finish();
  if (body !== null && response.length !== undefined) setStreamLength(body, response.length);
  return fetchedResponse(body, { status: response.status, statusText: response.statusText, headers: response.headers }, response.url, response.redirected);
}

function fetchAbortReason(signal: AbortSignal): Error {
  return signal.reason instanceof Error ? signal.reason : new Error(String(signal.reason));
}

export class Cache {
  #name: string;
  constructor(name: unknown = "default") { markHostObject(this); this.#name = string(name); }
  async match(request: unknown, options: unknown = {}): Promise<Response | undefined> {
    if (!propertyOf(options, "ignoreMethod") && requestMethodForCache(request) !== "GET") return undefined;
    const entry = await cacheMatch(this.#name, cacheKey(request, options));
    if (entry === undefined) return undefined;
    return new Response(entry.body, entry.metadata);
  }
  async matchAll(request: unknown, options: unknown = {}): Promise<Response[]> { const value = await this.match(request, options); return value ? [value] : []; }
  async put(request: unknown, response: unknown): Promise<void> {
    if (requestMethodForCache(request) !== "GET") throw new TypeError("Cache.put only accepts GET requests");
    if (!(response instanceof Response)) throw new TypeError("Cache.put requires a Response");
    const copy = response.clone();
    await cachePut(
      this.#name, cacheKey(request), {
        status: copy.status,
        statusText: copy.statusText,
        headers: [...copy.headers],
      }, new Uint8Array(await copy.arrayBuffer()),
    );
  }
  async delete(request: unknown, options: unknown = {}): Promise<boolean> {
    if (!propertyOf(options, "ignoreMethod") && requestMethodForCache(request) !== "GET") return false;
    return cacheDelete(this.#name, cacheKey(request, options));
  }
  async keys(): Promise<never> { throw cacheNotImplemented("Cache", "keys"); }
  async add(): Promise<never> { throw cacheNotImplemented("Cache", "add"); }
  async addAll(): Promise<never> { throw cacheNotImplemented("Cache", "addAll"); }
}

export class CacheStorage {
  declare default: Cache;
  #caches: Map<string, Cache>;
  constructor() {
    markHostObject(this);
    this.default = new Cache();
    this.#caches = new Map([["default", this.default]]);
  }
  async open(name: unknown = "default"): Promise<Cache> {
    const key = string(name);
    let cache = this.#caches.get(key);
    if (!cache) {
      cache = new Cache(key);
      this.#caches.set(key, cache);
    }
    return cache;
  }
  async delete(): Promise<never> { throw cacheNotImplemented("CacheStorage", "delete"); }
  async has(): Promise<never> { throw cacheNotImplemented("CacheStorage", "has"); }
  async keys(): Promise<never> { throw cacheNotImplemented("CacheStorage", "keys"); }
  async match(): Promise<never> { throw cacheNotImplemented("CacheStorage", "match"); }
}

function requestMethodForCache(request: unknown): string { return request instanceof Request ? request.method : "GET"; }
function cacheKey(request: unknown, options: unknown = {}): string {
  const url = new URL(request instanceof Request ? request.url : request);
  if (propertyOf(options, "ignoreSearch")) throw new Error("The 'ignoreSearch' field on 'CacheQueryOptions' is not implemented.");
  return url.href;
}
function cacheNotImplemented(type: string, method: string): Error { return new Error(`Failed to execute '${method}' on '${type}': the method is not implemented.`); }

const eventSourceStream = Symbol("event-source-stream");

export class EventSource extends EventTarget {
  declare static readonly CONNECTING: number;
  declare static readonly OPEN: number;
  declare static readonly CLOSED: number;
  #url: string;
  #withCredentials: boolean;
  #readyState = EventSource.CONNECTING;
  #onopen: unknown = null;
  #onmessage: unknown = null;
  #onerror: unknown = null;
  #reader: ReadableStreamDefaultReader<unknown> | null = null;
  #controller = new AbortController();
  #lastEventId = "";

  constructor(url: unknown, options: unknown = {}) {
    super();
    const stream = propertyOf(options, eventSourceStream);
    const fromStream = stream instanceof ReadableStream;
    this.#url = fromStream ? "" : new URL(url, globalThis.__tokamak_request?.url).href;
    this.#withCredentials = Boolean(propertyOf(options, "withCredentials"));
    if (fromStream) {
      this.#readyState = EventSource.OPEN;
      void this.#consume(stream);
    }
    else void this.#connect(options);
  }

  get url(): string { return this.#url; }
  get readyState(): number { return this.#readyState; }
  get withCredentials(): boolean { return this.#withCredentials; }
  get onopen(): unknown { return this.#onopen; }
  set onopen(value: unknown) { this.#onopen = value; }
  get onmessage(): unknown { return this.#onmessage; }
  set onmessage(value: unknown) { this.#onmessage = value; }
  get onerror(): unknown { return this.#onerror; }
  set onerror(value: unknown) { this.#onerror = value; }

  close(): void {
    if (this.#readyState === EventSource.CLOSED) return;
    this.#readyState = EventSource.CLOSED;
    this.#controller.abort();
    void this.#reader?.cancel().catch(() => {});
  }

  static from(stream: unknown): EventSource {
    if (!(stream instanceof ReadableStream)) throw new TypeError("EventSource.from requires a ReadableStream");
    return new EventSource("", { [eventSourceStream]: stream });
  }

  #emit(event: Event): void {
    this.dispatchEvent(event);
    const handler = propertyOf(this, "on" + event.type);
    if (isFunction(handler)) Reflect.apply(handler, this, [event]);
  }

  #fail(error: unknown): void {
    if (this.#readyState === EventSource.CLOSED) return;
    this.#readyState = EventSource.CONNECTING;
    this.#emit(new ErrorEvent("error", { error, message: propertyOf(error, "message") ?? String(error) }));
  }

  async #connect(options: unknown): Promise<void> {
    try {
      const fetcher = propertyOf(options, "fetcher") ?? globalThis;
      const fetchFunction = propertyOf(fetcher, "fetch");
      if (!isFunction(fetchFunction)) throw new TypeError("EventSource fetcher must provide fetch()");
      const response = await Reflect.apply(fetchFunction, fetcher, [this.#url, {
        headers: { accept: "text/event-stream" },
        signal: this.#controller.signal,
      }]);
      const body = propertyOf(response, "body");
      if (!propertyOf(response, "ok") || !(body instanceof ReadableStream)) throw new TypeError("EventSource response was not a stream");
      this.#readyState = EventSource.OPEN;
      this.#emit(new Event("open"));
      await this.#consume(body);
    } catch (error) {
      this.#fail(error);
    }
  }

  async #consume(stream: ReadableStream<unknown>): Promise<void> {
    const reader = stream.getReader();
    this.#reader = reader;
    const decoder = new TextDecoder();
    let input = "";
    let data: string[] = [];
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
    const line = (value: string) => {
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
        input = lines.pop() ?? "";
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
  for (const target of [EventSource, EventSource.prototype]) {
    Object.defineProperty(target, name, { value, writable: true, enumerable: true, configurable: true });
  }
}

export { bodyStream, bytes, consumeStream, hostResponse };
