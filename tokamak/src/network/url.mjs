import { markHostObject } from "../globals/objects.mjs";
import { urlDecodeParams, urlEncodeParams, urlParse, urlPatternCompile, urlPatternExec, urlPatternTest, urlSetComponent } from "tokamak:host";

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
      } else output += "�";
    } else if (code >= 0xdc00 && code <= 0xdfff) output += "�";
    else output += input[index];
  }
  return output;
}

export class URLSearchParams {
  constructor(init = "") {
    markHostObject(this);
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
  toString() { return urlEncodeParams(JSON.stringify(this.__values)); }
  [Symbol.iterator]() { return this.entries(); }
  get [Symbol.toStringTag]() { return "URLSearchParams"; }

}

function parseParams(params, input) {
  const value = input.startsWith("?") ? input.slice(1) : input;
  if (value === "") return;
  for (const pair of JSON.parse(urlDecodeParams(value))) params.__values.push(pair);
}

function paramsChanged(params) { params.__update?.(params.toString()); }
function connectParams(params, update) { params.__update = update; }
function replaceParams(params, input) { params.__values = []; parseParams(params, input); }

function applyComponent(url, component, value) {
  url.__parts = urlSetComponent(url.__parts.href, component, value);
}

export class URL {
  constructor(input, base) {
    markHostObject(this);
    hidden(this, "__parts", urlParse(usvString(input), base === undefined ? undefined : usvString(base)));
    const searchParams = new URLSearchParams(this.__parts.search);
    hidden(this, "__searchParams", searchParams);
    connectParams(searchParams, value => applyComponent(this, "search", value));
  }

  get protocol() { return this.__parts.protocol; }
  set protocol(value) { applyComponent(this, "protocol", usvString(value)); }
  get username() { return this.__parts.username; }
  set username(value) { applyComponent(this, "username", usvString(value)); }
  get password() { return this.__parts.password; }
  set password(value) { applyComponent(this, "password", usvString(value)); }
  get host() { return this.__parts.host; }
  set host(value) { applyComponent(this, "host", usvString(value)); }
  get hostname() { return this.__parts.hostname; }
  set hostname(value) { applyComponent(this, "hostname", usvString(value)); }
  get port() { return this.__parts.port; }
  set port(value) { applyComponent(this, "port", usvString(value)); }
  get pathname() { return this.__parts.pathname; }
  set pathname(value) { applyComponent(this, "pathname", usvString(value)); }
  get search() { return this.__parts.search; }
  set search(value) {
    applyComponent(this, "search", usvString(value));
    replaceParams(this.__searchParams, this.__parts.search);
  }
  get hash() { return this.__parts.hash; }
  set hash(value) { applyComponent(this, "hash", usvString(value)); }
  get href() { return this.__parts.href; }
  set href(value) {
    applyComponent(this, "href", usvString(value));
    replaceParams(this.__searchParams, this.__parts.search);
  }
  get origin() { return this.__parts.origin; }
  get searchParams() { return this.__searchParams; }
  toString() { return this.__parts.href; }
  toJSON() { return this.__parts.href; }
  get [Symbol.toStringTag]() { return "URL"; }
  static canParse(input, base) { try { new URL(input, base); return true; } catch { return false; } }
  static parse(input, base) { try { return new URL(input, base); } catch { return null; } }
  static createObjectURL(value) {
    if (value?.[blobBrand] !== true) throw new TypeError("The object is not a Blob");
    throw new Error("URL.createObjectURL is not supported");
  }
  static revokeObjectURL(value) { string(value); throw new Error("URL.revokeObjectURL is not supported"); }
}

const patternComponents = ["protocol", "username", "password", "hostname", "port", "pathname", "search", "hash"];

function patternPayload(input) {
  const value = input ?? {};
  if (typeof value === "string" || value instanceof String) return JSON.stringify(usvString(value));
  const init = {};
  for (const key of [...patternComponents, "baseURL"]) {
    if (value[key] !== undefined) init[key] = usvString(value[key]);
  }
  return JSON.stringify(init);
}

export class URLPattern {
  constructor(input = {}, baseOrOptions, options) {
    markHostObject(this);
    const baseIsString = typeof baseOrOptions === "string" || baseOrOptions instanceof String;
    const patternOptions = (baseIsString ? options : baseOrOptions) ?? {};
    hidden(this, "__pattern", patternPayload(input));
    hidden(this, "__baseURL", baseIsString ? usvString(baseOrOptions) : undefined);
    hidden(this, "__ignoreCase", patternOptions.ignoreCase === true);
    hidden(this, "__components", urlPatternCompile(this.__pattern, this.__baseURL, this.__ignoreCase));
  }

  get protocol() { return this.__components.protocol; }
  get username() { return this.__components.username; }
  get password() { return this.__components.password; }
  get hostname() { return this.__components.hostname; }
  get port() { return this.__components.port; }
  get pathname() { return this.__components.pathname; }
  get search() { return this.__components.search; }
  get hash() { return this.__components.hash; }
  get hasRegExpGroups() { return this.__components.hasRegExpGroups; }

  test(input = {}, baseURL) {
    return urlPatternTest(this.__pattern, this.__baseURL, this.__ignoreCase,
      patternPayload(input), baseURL === undefined ? undefined : usvString(baseURL));
  }
  get [Symbol.toStringTag]() { return "URLPattern"; }

  exec(input = {}, baseURL) {
    const match = urlPatternExec(this.__pattern, this.__baseURL, this.__ignoreCase,
      patternPayload(input), baseURL === undefined ? undefined : usvString(baseURL));
    if (!match) return null;
    const result = { inputs: baseURL === undefined ? [input] : [input, baseURL] };
    for (const component of patternComponents) result[component] = match[component];
    return result;
  }
}
