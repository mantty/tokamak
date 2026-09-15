import { TextDecoder, TextEncoder } from "../streams/text.mjs";

const encoder = new TextEncoder();
const decoder = new TextDecoder();
const specialProtocols = new Set(["file:", "ftp:", "http:", "https:", "ws:", "wss:"]);
const defaultPorts = new Map([
  ["http:", "80"],
  ["https:", "443"],
  ["ws:", "80"],
  ["wss:", "443"],
  ["ftp:", "21"],
]);
export const blobBrand = Symbol("tokamak.blob");

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

function hex(value) {
  const code = value.charCodeAt(0);
  return code >= 48 && code <= 57 ? code - 48
    : code >= 65 && code <= 70 ? code - 55
      : code >= 97 && code <= 102 ? code - 87 : -1;
}

function isPercentEscape(value, index) {
  return value[index] === "%" && index + 2 < value.length && hex(value[index + 1]) >= 0 && hex(value[index + 2]) >= 0;
}

function percentEncode(value, allowed, preserveEscapes = true) {
  const input = usvString(value);
  let output = "";
  for (let index = 0; index < input.length;) {
    if (preserveEscapes && isPercentEscape(input, index)) {
      output += input.slice(index, index + 3);
      index += 3;
      continue;
    }
    const character = String.fromCodePoint(input.codePointAt(index));
    const code = character.charCodeAt(0);
    if (character.length === 1 && ((code >= 48 && code <= 57) || (code >= 65 && code <= 90) || (code >= 97 && code <= 122) || allowed.includes(character))) {
      output += character;
    } else {
      for (const byte of encoder.encode(character)) output += "%" + byte.toString(16).toUpperCase().padStart(2, "0");
    }
    index += character.length;
  }
  return output;
}

function percentDecode(value, preserveInvalidPercent = true) {
  const input = usvString(value).replace(/\+/g, " ");
  const bytes = [];
  for (let index = 0; index < input.length;) {
    if (isPercentEscape(input, index)) {
      bytes.push(hex(input[index + 1]) * 16 + hex(input[index + 2]));
      index += 3;
      continue;
    }
    if (input[index] === "%" && !preserveInvalidPercent) {
      index += 1;
      continue;
    }
    const character = String.fromCodePoint(input.codePointAt(index));
    bytes.push(...encoder.encode(character));
    index += character.length;
  }
  return decoder.decode(new Uint8Array(bytes));
}

function formEncode(value) {
  return percentEncode(value, "*-._", false).replace(/%20/g, "+");
}

export class URLSearchParams {
  constructor(init = "") {
    hidden(this, "__values", []);
    hidden(this, "__update", null);
    if (init instanceof URLSearchParams) this.__values = init.__values.map(pair => pair.slice());
    else if (typeof init === "string" || init instanceof String) parseParams(this, string(init));
    else if (init != null && typeof init[Symbol.iterator] === "function") {
      for (const entry of init) {
        if (entry == null || typeof entry[Symbol.iterator] !== "function") throw new TypeError("Invalid URLSearchParams initializer");
        const pair = [...entry];
        if (pair.length !== 2) throw new TypeError("Invalid URLSearchParams initializer");
        this.__values.push([usvString(pair[0]), usvString(pair[1])]);
      }
    } else if (init != null && typeof init === "object") {
      for (const key of Object.keys(init)) this.__values.push([usvString(key), usvString(init[key])]);
    } else if (init != null) parseParams(this, string(init));
  }

  get size() { return this.__values.length; }
  append(name, value) { this.__values.push([usvString(name), usvString(value)]); paramsChanged(this); }
  delete(name, value) {
    const key = usvString(name);
    this.__values = value === undefined
      ? this.__values.filter(pair => pair[0] !== key)
      : this.__values.filter(pair => pair[0] !== key || pair[1] !== usvString(value));
    paramsChanged(this);
  }
  get(name) { return this.__values.find(pair => pair[0] === usvString(name))?.[1] ?? null; }
  getAll(name) { return this.__values.filter(pair => pair[0] === usvString(name)).map(pair => pair[1]); }
  has(name, value) {
    const key = usvString(name);
    return this.__values.some(pair => pair[0] === key && (value === undefined || pair[1] === usvString(value)));
  }
  set(name, value) {
    const key = usvString(name);
    const next = usvString(value);
    const index = this.__values.findIndex(pair => pair[0] === key);
    if (index < 0) this.__values.push([key, next]);
    else {
      this.__values[index][1] = next;
      this.__values = this.__values.filter((pair, pairIndex) => pair[0] !== key || pairIndex === index);
    }
    paramsChanged(this);
  }
  sort() {
    this.__values = this.__values.map((pair, index) => ({ pair, index }))
      .sort((left, right) => left.pair[0] < right.pair[0] ? -1 : left.pair[0] > right.pair[0] ? 1 : left.index - right.index)
      .map(entry => entry.pair);
    paramsChanged(this);
  }
  entries() { return this.__values.map(pair => pair.slice()).values(); }
  keys() { return this.__values.map(pair => pair[0]).values(); }
  values() { return this.__values.map(pair => pair[1]).values(); }
  forEach(callback, thisArg) { for (const [key, value] of this.__values) callback.call(thisArg, value, key, this); }
  toString() { return this.__values.map(pair => formEncode(pair[0]) + "=" + formEncode(pair[1])).join("&"); }
  [Symbol.iterator]() { return this.entries(); }
  get [Symbol.toStringTag]() { return "URLSearchParams"; }

}

function parseParams(params, input) {
  const value = input.startsWith("?") ? input.slice(1) : input;
  if (value === "") return;
  for (const pair of value.split("&")) {
    const separator = pair.indexOf("=");
    params.__values.push([
      percentDecode(separator < 0 ? pair : pair.slice(0, separator)),
      percentDecode(separator < 0 ? "" : pair.slice(separator + 1)),
    ]);
  }
}

function paramsChanged(params) { params.__update?.(params.toString()); }
function connectParams(params, update) { params.__update = update; }
function replaceParams(params, input) { params.__values = []; parseParams(params, input); }

function protocolMatch(value) {
  return /^([a-z][a-z\d+.-]*:)/i.exec(value);
}

function encodePath(value) { return percentEncode(value, "!$&'()*+,;=:@/"); }
function encodeQuery(value) { return percentEncode(value, "!$&'()*+,;=:@/?"); }
function encodeHash(value) { return percentEncode(value, "!$&'()*+,;=:@/?#"); }
function encodeUserInfo(value) { return percentEncode(value, "!$&'()*+,-.:;=_~"); }
function encodeOpaquePath(value) { return percentEncode(value, "!$&'()*+,;=:@/? "); }

function normalizePath(path) {
  const absolute = path.startsWith("/");
  const result = [];
  for (const segment of path.split("/")) {
    const dotSegment = segment.replace(/%2e/gi, ".");
    if (dotSegment === ".") continue;
    if (dotSegment === "..") {
      if (result.length > 0 && result[result.length - 1] !== "..") result.pop();
      else if (!absolute) result.push(segment);
    } else result.push(segment);
  }
  let value = result.join("/");
  if (absolute && !value.startsWith("/")) value = "/" + value;
  if (value === "" && absolute) value = "/";
  return value;
}

function parseAuthority(authority, protocol) {
  const at = authority.lastIndexOf("@");
  const credentials = at < 0 ? "" : authority.slice(0, at);
  const hostPort = at < 0 ? authority : authority.slice(at + 1);
  const separator = credentials.indexOf(":");
  const username = encodeUserInfo(separator < 0 ? credentials : credentials.slice(0, separator));
  const password = encodeUserInfo(separator < 0 ? "" : credentials.slice(separator + 1));
  let hostname = hostPort;
  let port = "";
  if (hostPort.startsWith("[")) {
    const end = hostPort.indexOf("]");
    if (end < 0) throw new TypeError("Invalid URL");
    hostname = hostPort.slice(0, end + 1).toLowerCase();
    if (hostPort.slice(end + 1) !== "") {
      if (hostPort[end + 1] !== ":") throw new TypeError("Invalid URL");
      port = hostPort.slice(end + 2);
    }
  } else {
    const colon = hostPort.lastIndexOf(":");
    if (colon >= 0 && hostPort.indexOf(":") === colon) {
      hostname = hostPort.slice(0, colon);
      port = hostPort.slice(colon + 1);
    }
    hostname = hostname.toLowerCase();
  }
  if (hostname === "" && protocol !== "file:") throw new TypeError("Invalid URL");
  if (specialProtocols.has(protocol)) {
    if (port !== "" && !/^\d+$/.test(port)) throw new TypeError("Invalid URL");
    if (port !== "" && Number(port) > 65535) throw new TypeError("Invalid URL");
  }
  if (port !== "" && /^\d+$/.test(port)) port = String(Number(port));
  if (port === defaultPorts.get(protocol)) port = "";
  return { username, password, hostname, port, host: port === "" ? hostname : hostname + ":" + port };
}

function splitSuffix(value) {
  const hashIndex = value.indexOf("#");
  const hashPresent = hashIndex >= 0;
  const withoutHash = hashPresent ? value.slice(0, hashIndex) : value;
  const queryIndex = withoutHash.indexOf("?");
  const searchPresent = queryIndex >= 0;
  return {
    main: searchPresent ? withoutHash.slice(0, queryIndex) : withoutHash,
    search: searchPresent ? withoutHash.slice(queryIndex + 1) : "",
    searchPresent,
    hash: hashPresent ? value.slice(hashIndex + 1) : "",
    hashPresent,
  };
}

function absoluteUrl(value) {
  const protocolValue = protocolMatch(value);
  if (!protocolValue) throw new TypeError("Invalid URL: " + value);
  const protocol = protocolValue[1].toLowerCase();
  const rest = value.slice(protocolValue[1].length);
  const suffix = splitSuffix(rest);
  let hasAuthority = suffix.main.startsWith("//");
  let authority = "";
  let pathname = suffix.main;
  if (hasAuthority) {
    const pathIndex = suffix.main.indexOf("/", 2);
    authority = pathIndex < 0 ? suffix.main.slice(2) : suffix.main.slice(2, pathIndex);
    pathname = pathIndex < 0 ? "/" : suffix.main.slice(pathIndex);
  } else if (protocol === "file:") {
    authority = "";
    pathname = suffix.main.startsWith("/") ? suffix.main : "/" + suffix.main;
    hasAuthority = true;
  } else if (specialProtocols.has(protocol)) {
    const specialPath = suffix.main.replace(/^\/+/, "");
    const pathIndex = specialPath.indexOf("/");
    authority = pathIndex < 0 ? specialPath : specialPath.slice(0, pathIndex);
    pathname = pathIndex < 0 ? "/" : specialPath.slice(pathIndex);
    hasAuthority = true;
  }
  const host = hasAuthority ? parseAuthority(authority, protocol) : { username: "", password: "", hostname: "", port: "", host: "" };
  if (hasAuthority) pathname = normalizePath(encodePath(pathname || "/"));
  else if (specialProtocols.has(protocol)) pathname = normalizePath(encodePath(pathname || "/"));
  else pathname = encodeOpaquePath(pathname);
  return {
    protocol,
    ...host,
    pathname,
    search: encodeQuery(suffix.search),
    searchPresent: suffix.searchPresent,
    hash: encodeHash(suffix.hash),
    hashPresent: suffix.hashPresent,
    hasAuthority,
  };
}

function copyParts(parts) { return { ...parts }; }

function relativeUrl(value, base) {
  if (specialProtocols.has(base.protocol)) value = value.replace(/\\/g, "/");
  if (value.startsWith("//")) return absoluteUrl(base.protocol + value);
  const suffix = splitSuffix(value);
  const path = suffix.main;
  const result = copyParts(base);
  if (path === "") {
    if (suffix.searchPresent) {
      result.search = encodeQuery(suffix.search);
      result.searchPresent = true;
    }
  } else {
    const merged = path.startsWith("/") ? path : base.pathname.slice(0, base.pathname.lastIndexOf("/") + 1) + path;
    result.pathname = normalizePath(encodePath(merged));
    result.search = suffix.searchPresent ? encodeQuery(suffix.search) : "";
    result.searchPresent = suffix.searchPresent;
    result.hash = "";
    result.hashPresent = false;
  }
  if (suffix.hashPresent) {
    result.hash = encodeHash(suffix.hash);
    result.hashPresent = true;
  } else if (path !== "" || suffix.searchPresent) {
    result.hash = "";
    result.hashPresent = false;
  }
  return result;
}

function parseUrl(input, base) {
  let value = usvString(input).trim();
  const protocol = protocolMatch(value)?.[1].toLowerCase();
  if (protocol && specialProtocols.has(protocol)) value = value.replace(/\\/g, "/");
  if (protocolMatch(value)) return absoluteUrl(value);
  if (base === undefined) throw new TypeError("Invalid URL: " + value);
  const parent = absoluteUrl(usvString(base).trim());
  return relativeUrl(value, parent);
}

function setUrl(url, parts) {
  hidden(url, "__parts", parts);
  const searchParams = new URLSearchParams(parts.search);
  hidden(url, "__searchParams", searchParams);
  hidden(url, "__origin", "");
  hidden(url, "__href", "");
  connectParams(searchParams, value => {
    url.__parts.search = value;
    url.__parts.searchPresent = value !== "";
    syncUrl(url);
  });
  syncUrl(url);
}

function syncUrl(url) {
  const parts = url.__parts;
  parts.host = parts.port === "" ? parts.hostname : parts.hostname + ":" + parts.port;
  url.__origin = parts.protocol !== "file:" && specialProtocols.has(parts.protocol) && parts.host !== "" ? parts.protocol + "//" + parts.host : "null";
  const credentials = parts.username || parts.password
    ? parts.username + (parts.password ? ":" + parts.password : "") + "@"
    : "";
  const authority = parts.hasAuthority ? "//" + credentials + parts.host : "";
  url.__href = parts.protocol + authority + parts.pathname
    + (parts.searchPresent ? "?" + parts.search : "")
    + (parts.hashPresent ? "#" + parts.hash : "");
}

function updateUrl(url, parts) {
  url.__parts = parts;
  replaceParams(url.__searchParams, parts.search);
  syncUrl(url);
}

export class URL {
  constructor(input, base) { setUrl(this, parseUrl(input, base)); }

  get protocol() { return this.__parts.protocol; }
  set protocol(value) {
    const next = usvString(value).toLowerCase().replace(/:$/, "") + ":";
    if (!/^[a-z][a-z\d+.-]*:$/.test(next)) return;
    this.__parts.protocol = next;
    if (this.__parts.port === defaultPorts.get(next)) this.__parts.port = "";
    syncUrl(this);
  }
  get username() { return this.__parts.username; }
  set username(value) { this.__parts.username = encodeUserInfo(value); syncUrl(this); }
  get password() { return this.__parts.password; }
  set password(value) { this.__parts.password = encodeUserInfo(value); syncUrl(this); }
  get host() { return this.__parts.host; }
  set host(value) {
    if (!this.__parts.hasAuthority) return;
    const host = parseAuthority(usvString(value), this.__parts.protocol);
    this.__parts.hostname = host.hostname;
    this.__parts.port = host.port;
    syncUrl(this);
  }
  get hostname() { return this.__parts.hostname; }
  set hostname(value) {
    if (!this.__parts.hasAuthority) return;
    const host = parseAuthority(usvString(value), this.__parts.protocol);
    this.__parts.hostname = host.hostname;
    this.__parts.port = this.__parts.port === defaultPorts.get(this.__parts.protocol) ? "" : this.__parts.port;
    syncUrl(this);
  }
  get port() { return this.__parts.port; }
  set port(value) {
    if (!this.__parts.hasAuthority) return;
    const next = usvString(value);
    if (next !== "" && (!/^\d+$/.test(next) || Number(next) > 65535)) return;
    const normalized = next === "" ? "" : String(Number(next));
    this.__parts.port = normalized === defaultPorts.get(this.__parts.protocol) ? "" : normalized;
    syncUrl(this);
  }
  get pathname() { return this.__parts.pathname; }
  set pathname(value) {
    this.__parts.pathname = this.__parts.hasAuthority ? normalizePath(encodePath(value) || "/") : encodeOpaquePath(value);
    syncUrl(this);
  }
  get search() { return this.__parts.search === "" ? "" : "?" + this.__parts.search; }
  set search(value) {
    const next = usvString(value);
    this.__parts.search = encodeQuery(next.startsWith("?") ? next.slice(1) : next);
    this.__parts.searchPresent = true;
    if (next === "") this.__parts.searchPresent = false;
    replaceParams(this.__searchParams, this.__parts.search);
    syncUrl(this);
  }
  get hash() { return this.__parts.hash === "" ? "" : "#" + this.__parts.hash; }
  set hash(value) {
    const next = usvString(value);
    this.__parts.hash = encodeHash(next.startsWith("#") ? next.slice(1) : next);
    this.__parts.hashPresent = next !== "";
    syncUrl(this);
  }
  get href() { return this.__href; }
  set href(value) { updateUrl(this, parseUrl(value)); }
  get origin() { return this.__origin; }
  get searchParams() { return this.__searchParams; }
  toString() { return this.href; }
  toJSON() { return this.href; }
  get [Symbol.toStringTag]() { return "URL"; }
  static canParse(input, base) { try { new URL(input, base); return true; } catch { return false; } }
  static parse(input, base) { try { return new URL(input, base); } catch { return null; } }
  static createObjectURL(value) {
    if (value?.[blobBrand] !== true) throw new TypeError("The object is not a Blob");
    throw new Error("URL.createObjectURL is not supported");
  }
  static revokeObjectURL(value) { string(value); throw new Error("URL.revokeObjectURL is not supported"); }
}

function escapePattern(value) { return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"); }

function readPatternGroup(pattern, start) {
  let depth = 0;
  let escaped = false;
  for (let index = start; index < pattern.length; index += 1) {
    const character = pattern[index];
    if (escaped) {
      escaped = false;
      continue;
    }
    if (character === "\\") {
      escaped = true;
      continue;
    }
    if (character === "(") depth += 1;
    else if (character === ")") {
      depth -= 1;
      if (depth === 0) return { value: pattern.slice(start + 1, index), end: index + 1 };
    }
  }
  throw new TypeError("Invalid URLPattern regexp");
}

function compilePattern(value, component) {
  const pattern = usvString(value);
  const names = [];
  let numeric = 0;
  let hasRegExpGroups = false;
  let source = "";
  for (let index = 0; index < pattern.length;) {
    if (pattern[index] === ":") {
      const match = /^:([A-Za-z0-9_]+)/.exec(pattern.slice(index));
      if (match) {
        names.push(match[1]);
        index += match[0].length;
        let regexp = null;
        if (pattern[index] === "(") {
          const group = readPatternGroup(pattern, index);
          regexp = group.value;
          index = group.end;
          hasRegExpGroups = true;
        }
        source += "(" + (regexp ?? (component === "hostname" ? "[^.:]+" : component === "pathname" ? "[^/]+" : ".+?")) + ")";
        continue;
      }
    }
    if (pattern[index] === "(") {
      const group = readPatternGroup(pattern, index);
      names.push(String(numeric++));
      source += "(" + group.value + ")";
      index = group.end;
      hasRegExpGroups = true;
      continue;
    }
    if (pattern[index] === "*") {
      const double = pattern[index + 1] === "*";
      names.push(String(numeric++));
      if (double) {
        source += "(.*)";
        index += 2;
      } else {
        source += component === "hostname" && pattern !== "*" ? "([^.]+)"
          : component === "pathname" && pattern !== "*" ? "([^/]+)" : "(.*)";
        index += 1;
      }
      continue;
    }
    const character = pattern[index];
    source += escapePattern(character);
    index += 1;
  }
  return { regex: new RegExp("^" + source + "$"), names, hasRegExpGroups };
}

function patternInput(input) {
  if (typeof input !== "string" && !(input instanceof String)) return input ?? {};
  const value = string(input);
  if (!protocolMatch(value)) {
    const suffix = splitSuffix(value);
    return {
      pathname: suffix.main || "*",
      ...(suffix.searchPresent ? { search: suffix.search } : {}),
      ...(suffix.hashPresent ? { hash: suffix.hash } : {}),
    };
  }
  const protocolIndex = value.indexOf(":");
  const suffix = splitSuffix(value.slice(protocolIndex + 1));
  const main = suffix.main;
  if (!main.startsWith("//")) return { protocol: value.slice(0, protocolIndex), pathname: main };
  const pathIndex = main.indexOf("/", 2);
  const authority = pathIndex < 0 ? main.slice(2) : main.slice(2, pathIndex);
  const at = authority.lastIndexOf("@");
  const credentials = at < 0 ? "" : authority.slice(0, at);
  const hostPort = at < 0 ? authority : authority.slice(at + 1);
  let hostname = hostPort;
  let port;
  if (hostPort.startsWith("[")) {
    const end = hostPort.indexOf("]");
    if (end < 0) throw new TypeError("Invalid URLPattern hostname");
    hostname = hostPort.slice(0, end + 1);
    if (hostPort[end + 1] === ":") port = hostPort.slice(end + 2);
  } else {
    const portSeparator = hostPort.lastIndexOf(":");
    if (portSeparator >= 0 && hostPort.indexOf(":") === portSeparator) {
      hostname = hostPort.slice(0, portSeparator);
      port = hostPort.slice(portSeparator + 1);
    }
  }
  return {
    protocol: value.slice(0, protocolIndex),
    ...(credentials ? { username: credentials } : {}),
    hostname,
    ...(port !== undefined ? { port } : {}),
    pathname: pathIndex < 0 ? "*" : main.slice(pathIndex),
    ...(suffix.searchPresent ? { search: suffix.search } : {}),
    ...(suffix.hashPresent ? { hash: suffix.hash } : {}),
  };
}

export class URLPattern {
  constructor(input = {}, baseURL) {
    if (baseURL === undefined && (typeof input === "string" || input instanceof String) && !protocolMatch(string(input)) && !string(input).startsWith("/")) {
      throw new TypeError("A base URL is required for a relative pattern");
    }
    let patterns = patternInput(input);
    if (baseURL !== undefined) {
      const base = new URL(baseURL);
      patterns = {
        protocol: base.protocol.slice(0, -1),
        username: base.username,
        password: base.password,
        hostname: base.hostname,
        port: base.port,
        ...patterns,
      };
      if (patterns.pathname && !patterns.pathname.startsWith("/")) patterns.pathname = "/" + patterns.pathname;
    }
    hidden(this, "__protocol", String(patterns.protocol ?? "*").replace(/:$/, ""));
    hidden(this, "__username", String(patterns.username ?? "*"));
    hidden(this, "__password", String(patterns.password ?? "*"));
    hidden(this, "__hostname", String(patterns.hostname ?? "*"));
    hidden(this, "__port", String(patterns.port ?? "*"));
    hidden(this, "__pathname", String(patterns.pathname ?? "*"));
    hidden(this, "__search", String(patterns.search ?? "*"));
    hidden(this, "__hash", String(patterns.hash ?? "*"));
    const components = Object.fromEntries(["protocol", "username", "password", "hostname", "port", "pathname", "search", "hash"]
      .map(component => [component, compilePattern(this[component], component)]));
    hidden(this, "__components", components);
    hidden(this, "__hasRegExpGroups", Object.values(components).some(component => component.hasRegExpGroups));
  }

  get protocol() { return this.__protocol; }
  get username() { return this.__username; }
  get password() { return this.__password; }
  get hostname() { return this.__hostname; }
  get port() { return this.__port; }
  get pathname() { return this.__pathname; }
  get search() { return this.__search; }
  get hash() { return this.__hash; }
  get hasRegExpGroups() { return this.__hasRegExpGroups; }

  test(input, baseURL) { return this.exec(input, baseURL) !== null; }
  get [Symbol.toStringTag]() { return "URLPattern"; }

  exec(input, baseURL) {
    let url;
    try { url = input instanceof URL ? input : new URL(string(input), baseURL); }
    catch { return null; }
    const values = {
      protocol: url.protocol.slice(0, -1), username: url.username, password: url.password,
      hostname: url.hostname, port: url.port, pathname: url.pathname, search: url.search.slice(1), hash: url.hash.slice(1),
    };
    const result = {};
    for (const component of Object.keys(values)) {
      const { regex, names } = this.__components[component];
      const match = regex.exec(values[component]);
      if (!match) return null;
      const groups = {};
      for (let index = 0; index < names.length; index += 1) groups[names[index]] = match[index + 1] ?? "";
      result[component] = { input: values[component], groups };
    }
    const inputs = baseURL === undefined ? [input] : [input, baseURL];
    return { inputs, protocol: result.protocol, username: result.username, password: result.password, hostname: result.hostname, port: result.port, pathname: result.pathname, search: result.search, hash: result.hash };
  }
}
