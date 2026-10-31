import { callFunction, hostObjectKinds, isIterable, isObject, markHostObject, objectOf, propertyOf } from "../globals/objects.ts";
import {
  urlDecodeParams, urlEncodeParams, urlParse, urlPatternCompile, urlPatternMatchInput, urlSetComponent,
  type CompiledPattern, type UrlParts,
} from "tokamak:host";
import { string, usvString } from "../globals/conversions.ts";

type Pair = [string, string];

// Set by URLSearchParams for URL, which keeps its searchParams in step with
// its search.
let attachParams: (params: URLSearchParams, update: (value: string) => void) => void;
let replaceParams: (params: URLSearchParams, input: string) => void;

function decodeParams(input: string): Pair[] {
  const value = input.startsWith("?") ? input.slice(1) : input;
  return value === "" ? [] : urlDecodeParams(value);
}

function initPair(entry: unknown): Pair {
  if (!isIterable(entry)) throw new TypeError("Invalid URLSearchParams initializer");
  const pair = [...entry];
  if (pair.length !== 2) throw new TypeError("Invalid URLSearchParams initializer");
  return [usvString(pair[0]), usvString(pair[1])];
}

function initPairs(init: unknown): Pair[] {
  if (typeof init === "string" || init instanceof String) return decodeParams(string(init));
  if (isIterable(init)) return Array.from(init, initPair);
  if (typeof init === "object" && init !== null) {
    const record = objectOf(init);
    return Object.keys(record).map((key): Pair => [usvString(key), usvString(record[key])]);
  }
  return init == null ? [] : decodeParams(string(init));
}

export class URLSearchParams {
  #values: Pair[];
  #update: ((value: string) => void) | null = null;

  constructor(init: unknown = "") {
    markHostObject(this);
    this.#values = init instanceof URLSearchParams ? init.#values.map((pair): Pair => [pair[0], pair[1]]) : initPairs(init);
  }

  #changed(): void { this.#update?.(this.toString()); }

  get size(): number { return this.#values.length; }
  append(name: unknown, value: unknown): void { this.#values.push([usvString(name), usvString(value)]); this.#changed(); }
  delete(name: unknown, value?: unknown): void {
    const key = usvString(name);
    this.#values = value === undefined
      ? this.#values.filter(pair => pair[0] !== key)
      : this.#values.filter(pair => pair[0] !== key || pair[1] !== usvString(value));
    this.#changed();
  }
  get(name: unknown): string | null { return this.#values.find(pair => pair[0] === usvString(name))?.[1] ?? null; }
  getAll(name: unknown): string[] { return this.#values.filter(pair => pair[0] === usvString(name)).map(pair => pair[1]); }
  has(name: unknown, value?: unknown): boolean {
    const key = usvString(name);
    return this.#values.some(pair => pair[0] === key && (value === undefined || pair[1] === usvString(value)));
  }
  set(name: unknown, value: unknown): void {
    const key = usvString(name);
    const next = usvString(value);
    const index = this.#values.findIndex(pair => pair[0] === key);
    if (index < 0) this.#values.push([key, next]);
    else {
      this.#values[index][1] = next;
      this.#values = this.#values.filter((pair, pairIndex) => pair[0] !== key || pairIndex === index);
    }
    this.#changed();
  }
  sort(): void {
    this.#values = this.#values.map((pair, index) => ({ pair, index }))
      .sort((left, right) => left.pair[0] < right.pair[0] ? -1 : left.pair[0] > right.pair[0] ? 1 : left.index - right.index)
      .map(entry => entry.pair);
    this.#changed();
  }
  entries(): ArrayIterator<Pair> { return this.#values.map((pair): Pair => [pair[0], pair[1]]).values(); }
  keys(): ArrayIterator<string> { return this.#values.map(pair => pair[0]).values(); }
  values(): ArrayIterator<string> { return this.#values.map(pair => pair[1]).values(); }
  forEach(callback: unknown, thisArg?: unknown): void {
    for (const [key, value] of this.#values) callFunction(callback, thisArg, value, key, this);
  }
  toString(): string { return urlEncodeParams(this.#values); }
  [Symbol.iterator](): ArrayIterator<Pair> { return this.entries(); }
  get [Symbol.toStringTag](): string { return "URLSearchParams"; }

  static {
    attachParams = (params, update) => { params.#update = update; };
    replaceParams = (params, input) => { params.#values = decodeParams(input); };
  }
}

export class URL {
  #parts: UrlParts;
  readonly #searchParams: URLSearchParams;

  constructor(input: unknown, base?: unknown) {
    markHostObject(this);
    this.#parts = urlParse(usvString(input), base === undefined ? undefined : usvString(base));
    this.#searchParams = new URLSearchParams(this.#parts.search);
    attachParams(this.#searchParams, value => { this.#apply("search", value); });
  }

  #apply(component: string, value: string): void {
    this.#parts = urlSetComponent(this.#parts.href, component, value);
  }

  get protocol(): string { return this.#parts.protocol; }
  set protocol(value: unknown) { this.#apply("protocol", usvString(value)); }
  get username(): string { return this.#parts.username; }
  set username(value: unknown) { this.#apply("username", usvString(value)); }
  get password(): string { return this.#parts.password; }
  set password(value: unknown) { this.#apply("password", usvString(value)); }
  get host(): string { return this.#parts.host; }
  set host(value: unknown) { this.#apply("host", usvString(value)); }
  get hostname(): string { return this.#parts.hostname; }
  set hostname(value: unknown) { this.#apply("hostname", usvString(value)); }
  get port(): string { return this.#parts.port; }
  set port(value: unknown) { this.#apply("port", usvString(value)); }
  get pathname(): string { return this.#parts.pathname; }
  set pathname(value: unknown) { this.#apply("pathname", usvString(value)); }
  get search(): string { return this.#parts.search; }
  set search(value: unknown) {
    this.#apply("search", usvString(value));
    replaceParams(this.#searchParams, this.#parts.search);
  }
  get hash(): string { return this.#parts.hash; }
  set hash(value: unknown) { this.#apply("hash", usvString(value)); }
  get href(): string { return this.#parts.href; }
  set href(value: unknown) {
    this.#apply("href", usvString(value));
    replaceParams(this.#searchParams, this.#parts.search);
  }
  get origin(): string { return this.#parts.origin; }
  get searchParams(): URLSearchParams { return this.#searchParams; }
  toString(): string { return this.#parts.href; }
  toJSON(): string { return this.#parts.href; }
  get [Symbol.toStringTag](): string { return "URL"; }
  static canParse(input: unknown, base?: unknown): boolean { try { new URL(input, base); return true; } catch { return false; } }
  static parse(input: unknown, base?: unknown): URL | null { try { return new URL(input, base); } catch { return null; } }
  static createObjectURL(value: unknown): never {
    const kind = isObject(value) ? hostObjectKinds.get(value) : undefined;
    if (kind !== "Blob" && kind !== "File") throw new TypeError("The object is not a Blob");
    throw new Error("URL.createObjectURL is not supported");
  }
  static revokeObjectURL(value: unknown): never { string(value); throw new Error("URL.revokeObjectURL is not supported"); }
}

const patternComponents = ["protocol", "username", "password", "hostname", "port", "pathname", "search", "hash"] as const;

interface ComponentResult {
  input: string;
  groups: Record<string, string | undefined>;
}

function patternPayload(input: unknown): string {
  if (typeof input === "string" || input instanceof String) return JSON.stringify(usvString(input));
  const init: Record<string, string> = {};
  for (const key of [...patternComponents, "baseURL"]) {
    const value = propertyOf(input, key);
    if (value !== undefined) init[key] = usvString(value);
  }
  return JSON.stringify(init);
}

// URLPattern runs the engine's regular expressions through intrinsics captured
// before app code runs, so app code cannot intercept them.
const NativeRegExp = RegExp;
const regexpExec = RegExp.prototype.exec;
const apply = Reflect.apply;

// Runs `source` with `flags` on `input` for the host's pattern parser: the
// start and end of the match and each capture, or undefined without a match.
function runRegExp(source: string, flags: string, input: string): RegExpIndicesArray | undefined {
  return apply(regexpExec, new NativeRegExp(source, `${flags}d`), [input])?.indices;
}

export class URLPattern {
  readonly #pattern: CompiledPattern;
  // Each component's regular expression, compiled when it first matches.
  readonly #regexps: (RegExp | undefined)[] = [];

  constructor(input: unknown = {}, baseOrOptions?: unknown, options?: unknown) {
    markHostObject(this);
    const baseIsString = typeof baseOrOptions === "string" || baseOrOptions instanceof String;
    const ignoreCase = propertyOf((baseIsString ? options : baseOrOptions) ?? {}, "ignoreCase") === true;
    const base = baseIsString ? usvString(baseOrOptions) : undefined;
    this.#pattern = urlPatternCompile(patternPayload(input), base, ignoreCase, runRegExp);
  }

  get protocol(): string { return this.#pattern.protocol.pattern; }
  get username(): string { return this.#pattern.username.pattern; }
  get password(): string { return this.#pattern.password.pattern; }
  get hostname(): string { return this.#pattern.hostname.pattern; }
  get port(): string { return this.#pattern.port.pattern; }
  get pathname(): string { return this.#pattern.pathname.pattern; }
  get search(): string { return this.#pattern.search.pattern; }
  get hash(): string { return this.#pattern.hash.pattern; }
  get hasRegExpGroups(): boolean { return this.#pattern.hasRegExpGroups; }

  test(input: unknown = {}, baseURL?: unknown): boolean { return this.#exec(input, baseURL) !== null; }
  get [Symbol.toStringTag](): string { return "URLPattern"; }

  exec(input: unknown = {}, baseURL?: unknown): Record<string, unknown> | null { return this.#exec(input, baseURL); }

  // The match of this pattern on `input`, or null.
  #exec(input: unknown, baseURL: unknown): Record<string, unknown> | null {
    const inputs = urlPatternMatchInput(patternPayload(input), baseURL === undefined ? undefined : usvString(baseURL));
    if (!inputs) return null;
    const result: Record<string, unknown> = { inputs: baseURL === undefined ? [input] : [input, baseURL] };
    for (let index = 0; index < patternComponents.length; index += 1) {
      const name = patternComponents[index];
      const component = this.#pattern[name];
      const regexp = this.#regexps[index] ??= new NativeRegExp(component.regexp, component.flags);
      const match = componentMatch(regexp, component.groups, inputs[name]);
      if (!match) return null;
      result[name] = match;
    }
    return result;
  }
}

// The match of a component's `regexp` on `input`, with its `groups` by name, or null.
function componentMatch(regexp: RegExp, groups: readonly string[], input: string): ComponentResult | null {
  const result = apply(regexpExec, regexp, [input]);
  if (result === null) return null;
  const matched: Record<string, string | undefined> = {};
  for (let group = 0; group < groups.length; group += 1) matched[groups[group]] = result[group + 1];
  return { input, groups: matched };
}
