import { string } from "../globals/conversions.ts";
import { objectOf } from "../globals/objects.ts";
import { URL, URLSearchParams } from "../network/url.ts";
import { parse as parseQuery, stringify as stringifyQuery } from "./querystring.ts";
import { toASCII, toUnicode } from "./punycode.ts";
import { codedError } from "./errors.ts";

export { URL, URLSearchParams };

function queryValue(value: unknown): string {
  if (value === undefined || value === null) return "";
  return typeof value === "string" ? value : stringifyQuery(value);
}

function parseRelative(value: unknown, parseQueryString: unknown, slashesDenoteHost: unknown): Url {
  const output = new Url();
  const input = string(value);
  const hashIndex = input.indexOf("#");
  const withoutHash = hashIndex < 0 ? input : input.slice(0, hashIndex);
  output.hash = hashIndex < 0 ? null : input.slice(hashIndex);
  const searchIndex = withoutHash.indexOf("?");
  const pathname = searchIndex < 0 ? withoutHash : withoutHash.slice(0, searchIndex);
  const query = searchIndex < 0 ? null : withoutHash.slice(searchIndex + 1);
  output.protocol = null;
  output.slashes = null;
  output.auth = null;
  output.host = null;
  output.port = null;
  output.hostname = null;
  output.pathname = pathname || null;
  output.search = query === null ? null : `?${query}`;
  output.query = parseQueryString ? parseQuery(query ?? "") : query;
  output.path = output.pathname === null ? output.search : `${output.pathname}${output.search ?? ""}`;
  output.href = input;
  if (slashesDenoteHost && input.startsWith("//")) {
    const parsed = new URL(`http:${input}`);
    output.slashes = true;
    output.host = parsed.host;
    output.hostname = parsed.hostname;
    output.port = parsed.port || null;
    output.pathname = parsed.pathname;
    output.search = parsed.search || null;
    output.query = parseQueryString ? parseQuery(parsed.search.slice(1)) : parsed.search.slice(1);
    output.hash = parsed.hash || null;
    output.path = output.pathname + (output.search ?? "");
  }
  return output;
}

// A URL's parts, which `parse` sets and `format` reads; a `Url` from the
// constructor has none.
export class Url {
  declare protocol?: string | null;
  declare slashes?: boolean | null;
  declare auth?: string | null;
  declare host?: string | null;
  declare port?: string | null;
  declare hostname?: string | null;
  declare hash?: string | null;
  declare search?: string | null;
  declare query?: string | null | Record<PropertyKey, unknown>;
  declare pathname?: string | null;
  declare path?: string | null;
  declare href?: string;

  parse(value: unknown, parseQueryString: unknown = false, slashesDenoteHost: unknown = false): Url {
    const input = string(value);
    const protocol = /^[A-Za-z][A-Za-z\d+.-]*:/.test(input);
    if (!protocol && !(slashesDenoteHost && input.startsWith("//"))) return parseRelative(input, parseQueryString, slashesDenoteHost);
    const parsed = new URL(input, protocol ? undefined : "http://localhost");
    const output = new Url();
    output.protocol = parsed.protocol || null;
    output.slashes = input.includes("//") ? true : null;
    output.auth = urlAuth(parsed);
    output.host = parsed.host || null;
    output.port = parsed.port || null;
    output.hostname = parsed.hostname || null;
    output.pathname = parsed.pathname || null;
    output.search = parsed.search || null;
    output.query = parseQueryString ? parseQuery(parsed.search.slice(1)) : parsed.search.slice(1);
    output.hash = parsed.hash || null;
    output.path = output.pathname === null ? output.search : `${output.pathname}${output.search ?? ""}`;
    output.href = parsed.href;
    return output;
  }

  format(value: unknown): string { return format(value); }
  resolve(from: unknown, to: unknown): string { return resolveUrl(from, to); }
  resolveObject(from: unknown, to: unknown): Url { return new Url().parse(this.resolve(from, to)); }
  toString(): string { return this.href ?? format(this); }
}

function decodePart(value: string): string {
  try { return decodeURIComponent(value); }
  catch { return value; }
}

function urlAuth(url: URL): string | null {
  return url.username || url.password ? `${decodePart(url.username)}${url.password ? `:${decodePart(url.password)}` : ""}` : null;
}

export function parse(value: unknown, parseQueryString: unknown = false, slashesDenoteHost: unknown = false): Url {
  return new Url().parse(value, parseQueryString, slashesDenoteHost);
}

// The `?search` that a URL object's `search`, or else its `query`, gives.
function searchOf(parts: Record<PropertyKey, unknown>): string {
  if (parts.search !== undefined && parts.search !== null) {
    const search = string(parts.search);
    return search === "" || search.startsWith("?") ? search : `?${search}`;
  }
  const query = parts.query === undefined || parts.query === null ? "" : queryValue(parts.query);
  return query ? `?${query}` : "";
}

export function format(value: unknown): string {
  if (value instanceof URL) return value.href;
  if (value === null || value === undefined) throw new TypeError("The url argument must be of type string or an object");
  if (typeof value === "string") return value;
  const parts = objectOf(value);
  const protocol = parts.protocol ? string(parts.protocol).replace(/:$/, "") + ":" : "";
  const auth = parts.auth ? encodeURIComponent(string(parts.auth)).replace(/%3A/i, ":") + "@" : "";
  const hostname = parts.hostname ? string(parts.hostname) : "";
  const port = parts.port ? `:${string(parts.port)}` : "";
  const host = string(parts.host ?? (hostname ? `${hostname.includes(":") ? `[${hostname}]` : hostname}${port}` : ""));
  let pathname = string(parts.pathname ?? "");
  if (host && pathname && !pathname.startsWith("/")) pathname = "/" + pathname;
  const hash = parts.hash ? (string(parts.hash).startsWith("#") ? string(parts.hash) : `#${string(parts.hash)}`) : "";
  const slashes = parts.slashes || host || ["http:", "https:", "ftp:", "gopher:", "file:"].includes(protocol);
  return protocol + (slashes ? "//" : "") + auth + host + pathname + searchOf(parts) + hash;
}

function normalizeDotPath(pathname: string): string {
  const trailing = pathname.endsWith("/");
  const parts: string[] = [];
  for (const part of pathname.split("/")) {
    if (part === "" || part === ".") continue;
    if (part === "..") { parts.pop(); continue; }
    parts.push(part);
  }
  const normalized = "/" + parts.join("/");
  return trailing && normalized !== "/" ? normalized + "/" : normalized;
}

function resolveUrl(from: unknown, to: unknown): string {
  const input = string(to);
  const base = new URL(string(from));
  const suffixIndex = input.search(/[?#]/);
  const path = suffixIndex < 0 ? input : input.slice(0, suffixIndex);
  const suffix = suffixIndex < 0 ? "" : input.slice(suffixIndex);
  const isAbsoluteUrl = /^[A-Za-z][A-Za-z\d+.-]*:\/\//.test(input);
  const isProtocolRelative = input.startsWith("//");
  if (!isAbsoluteUrl && !isProtocolRelative && path !== "") {
    const merged = path.startsWith("/") ? path : base.pathname.slice(0, base.pathname.lastIndexOf("/") + 1) + path;
    return new URL(normalizeDotPath(merged) + suffix, string(from)).href;
  }
  if (isAbsoluteUrl) {
    const match = input.match(/^([A-Za-z][A-Za-z\d+.-]*:\/\/[^/?#]*)([^?#]*)([?#].*)?$/);
    if (match) return new URL(match[1] + normalizeDotPath(match[2] || "/") + (match[3] || "")).href;
  }
  if (isProtocolRelative) {
    const match = input.match(/^(\/\/[^/?#]*)([^?#]*)([?#].*)?$/);
    if (match) return new URL(match[1] + normalizeDotPath(match[2] || "/") + (match[3] || ""), string(from)).href;
  }
  return new URL(input, string(from)).href;
}

export function resolve(from: unknown, to: unknown): string { return resolveUrl(from, to); }
export function resolveObject(from: unknown, to: unknown): Url { return new Url().resolveObject(from, to); }

export function domainToASCII(value: unknown): string { return toASCII(value); }
export function domainToUnicode(value: unknown): string { return toUnicode(value); }

export function fileURLToPath(value: unknown): string {
  const url = value instanceof URL ? value : new URL(string(value));
  if (url.protocol !== "file:") throw codedError(TypeError, "The URL must be of scheme file", "ERR_INVALID_URL_SCHEME");
  if (url.hostname && url.hostname !== "localhost") throw codedError(TypeError, "File URL host must be \"localhost\" or empty", "ERR_INVALID_FILE_URL_HOST");
  return decodeURIComponent(url.pathname);
}

export function pathToFileURL(value: unknown): URL {
  const input = string(value).replaceAll("\\", "/");
  const path = input.startsWith("/") ? input : `/${input}`;
  const encoded = path.split("/").map(part => encodeURIComponent(part)).join("/");
  return new URL(`file://${encoded}`);
}

export function toPathIfFileURL(value: unknown): unknown {
  if (value instanceof URL && value.protocol === "file:") return fileURLToPath(value);
  return value;
}

// The options of an HTTP request to a URL, as `http.request` takes them.
interface HttpOptions {
  protocol: string;
  hostname: string;
  hash: string;
  search: string;
  pathname: string;
  path: string;
  href: string;
  port?: number;
  auth?: string;
}

export function urlToHttpOptions(value: unknown): HttpOptions {
  const url = value instanceof URL ? value : new URL(string(value));
  const options: HttpOptions = {
    protocol: url.protocol,
    hostname: url.hostname,
    hash: url.hash,
    search: url.search,
    pathname: url.pathname,
    path: url.pathname + url.search,
    href: url.href,
  };
  if (url.port) options.port = Number(url.port);
  const auth = urlAuth(url);
  if (auth !== null) options.auth = auth;
  return options;
}

export default { URL, URLSearchParams, Url, parse, format, resolve, resolveObject, domainToASCII, domainToUnicode, fileURLToPath, pathToFileURL, toPathIfFileURL, urlToHttpOptions };
