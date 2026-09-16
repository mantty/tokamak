import { URL, URLSearchParams } from "../network/url.mjs";
import { parse as parseQuery, stringify as stringifyQuery } from "./querystring.mjs";
import { toASCII, toUnicode } from "./punycode.mjs";

export { URL, URLSearchParams };

function string(value) {
  if (typeof value === "symbol") throw new TypeError("Cannot convert a Symbol value to a string");
  return String(value);
}

function queryValue(value) {
  if (value === undefined || value === null) return "";
  return typeof value === "string" ? value : stringifyQuery(value);
}

function emptyUrl() {
  return Object.create(Url.prototype);
}

function parseRelative(value, parseQueryString, slashesDenoteHost) {
  const output = emptyUrl();
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

export class Url {
  parse(value, parseQueryString = false, slashesDenoteHost = false) {
    const input = string(value);
    const protocol = /^[A-Za-z][A-Za-z\d+.-]*:/.test(input);
    if (!protocol && !(slashesDenoteHost && input.startsWith("//"))) return parseRelative(input, parseQueryString, slashesDenoteHost);
    const parsed = new URL(input, protocol ? undefined : "http://localhost");
    const output = emptyUrl();
    output.protocol = parsed.protocol || null;
    output.slashes = input.includes("//") ? true : null;
    output.auth = parsed.username || parsed.password ? `${decodePart(parsed.username)}${parsed.password ? `:${decodePart(parsed.password)}` : ""}` : null;
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

  format(value) { return format(value); }
  resolve(from, to) { return resolveUrl(from, to); }
  resolveObject(from, to) { return new Url().parse(this.resolve(from, to)); }
  toString() { return this.href ?? format(this); }
}

function decodePart(value) {
  try { return decodeURIComponent(value); }
  catch { return value; }
}

export function parse(value, parseQueryString = false, slashesDenoteHost = false) {
  return new Url().parse(value, parseQueryString, slashesDenoteHost);
}

export function format(value) {
  if (value instanceof URL) return value.href;
  if (value === null || value === undefined) throw new TypeError("The url argument must be of type string or an object");
  if (typeof value === "string") return value;
  const protocol = value.protocol ? String(value.protocol).replace(/:$/, "") + ":" : "";
  const auth = value.auth ? encodeURIComponent(String(value.auth)).replace(/%3A/i, ":") + "@" : "";
  const hostname = value.hostname ? String(value.hostname) : "";
  const host = value.host ?? (hostname ? `${hostname.includes(":") ? `[${hostname}]` : hostname}${value.port ? `:${value.port}` : ""}` : "");
  let pathname = value.pathname ?? "";
  if (host && pathname && !pathname.startsWith("/")) pathname = "/" + pathname;
  const search = value.search !== undefined && value.search !== null
    ? (String(value.search) === "" ? "" : (String(value.search).startsWith("?") ? String(value.search) : `?${value.search}`))
    : (value.query !== undefined && value.query !== null ? (queryValue(value.query) ? `?${queryValue(value.query)}` : "") : "");
  const hash = value.hash ? (String(value.hash).startsWith("#") ? String(value.hash) : `#${value.hash}`) : "";
  const slashes = value.slashes || host || ["http:", "https:", "ftp:", "gopher:", "file:"].includes(protocol);
  return protocol + (slashes ? "//" : "") + auth + host + pathname + search + hash;
}

function normalizeDotPath(pathname) {
  const trailing = pathname.endsWith("/");
  const parts = [];
  for (const part of pathname.split("/")) {
    if (part === "" || part === ".") continue;
    if (part === "..") { parts.pop(); continue; }
    parts.push(part);
  }
  const normalized = "/" + parts.join("/");
  return trailing && normalized !== "/" ? normalized + "/" : normalized;
}

function resolveUrl(from, to) {
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

export function resolve(from, to) { return resolveUrl(from, to); }
export function resolveObject(from, to) { return new Url().resolveObject(from, to); }

export function domainToASCII(value) { return toASCII(value); }
export function domainToUnicode(value) { return toUnicode(value); }

export function fileURLToPath(value) {
  const url = value instanceof URL ? value : new URL(string(value));
  if (url.protocol !== "file:") { const error = new TypeError("The URL must be of scheme file"); error.code = "ERR_INVALID_URL_SCHEME"; throw error; }
  if (url.hostname && url.hostname !== "localhost") { const error = new TypeError("File URL host must be \"localhost\" or empty"); error.code = "ERR_INVALID_FILE_URL_HOST"; throw error; }
  return decodeURIComponent(url.pathname);
}

export function pathToFileURL(value) {
  const input = string(value).replaceAll("\\", "/");
  const path = input.startsWith("/") ? input : `/${input}`;
  const encoded = path.split("/").map(part => encodeURIComponent(part)).join("/");
  return new URL(`file://${encoded}`);
}

export function toPathIfFileURL(value) {
  if (value instanceof URL && value.protocol === "file:") return fileURLToPath(value);
  return value;
}

export function urlToHttpOptions(value) {
  const url = value instanceof URL ? value : new URL(string(value));
  const options = {
    protocol: url.protocol,
    hostname: url.hostname,
    hash: url.hash,
    search: url.search,
    pathname: url.pathname,
    path: url.pathname + url.search,
    href: url.href,
  };
  if (url.port) options.port = Number(url.port);
  if (url.username || url.password) options.auth = `${decodePart(url.username)}${url.password ? `:${decodePart(url.password)}` : ""}`;
  return options;
}

export default { URL, URLSearchParams, Url, parse, format, resolve, resolveObject, domainToASCII, domainToUnicode, fileURLToPath, pathToFileURL, toPathIfFileURL, urlToHttpOptions };
