import * as codes from "./dns-codes.ts";
import { codedError, type CodedError } from "./errors.ts";
import { isFunction, propertyOf, type AnyFunction } from "../globals/objects.ts";

export * from "./dns-codes.ts";

const dohEndpoint = "https://1.1.1.1/dns-query";
const typeNumbers = new Map([
  ["A", 1], ["NS", 2], ["CNAME", 5], ["SOA", 6], ["PTR", 12], ["MX", 15], ["TXT", 16], ["AAAA", 28], ["SRV", 33], ["NAPTR", 35], ["CAA", 257],
]);

// A record's value: an address or name, a TXT record's strings, or a record's fields.
type RecordValue = string | string[] | Record<string, string | number>;

function dnsError(name: string, code = "ENOTFOUND"): CodedError {
  return Object.assign(new Error(`query ${name} ${code}`), { code, hostname: name, syscall: "queryA" });
}

function unsupported(name: string): CodedError {
  const error = codedError(Error, `node:dns ${name} is not implemented in the Workers runtime`, "ERR_METHOD_NOT_IMPLEMENTED");
  return Object.assign(error, { syscall: name });
}

// The name of an answer's record type, from the resolver's JSON.
function typeName(answer: unknown): string | undefined {
  const type = propertyOf(answer, "type");
  return [...typeNumbers].find(([, number]) => number === type)?.[0];
}

function queryType(type: unknown): string {
  const value = String(type).toUpperCase();
  if (typeNumbers.get(value) === undefined) throw new TypeError(`Unknown DNS record type: ${String(type)}`);
  return value;
}

function validateName(name: unknown): string {
  if (typeof name !== "string" || name.length === 0) throw new TypeError("The hostname must be a non-empty string");
  return name;
}

function quoted(value: string): string[] {
  const parts = [];
  const pattern = /"((?:\\.|[^"\\])*)"/g;
  let match;
  while ((match = pattern.exec(value))) parts.push(match[1].replace(/\\([\\"])/g, "$1"));
  return parts.length ? parts : [value];
}

// An answer's text, from the resolver's JSON.
function answerData(answer: unknown): string {
  return String(propertyOf(answer, "data") ?? "");
}

function answerTtl(answer: unknown): number {
  return Number(propertyOf(answer, "TTL") ?? 0);
}

function answerValue(answer: unknown): RecordValue {
  const type = typeName(answer) ?? propertyOf(answer, "type");
  const data = answerData(answer).replace(/\.$/, "");
  switch (type) {
    case "A":
    case "AAAA":
    case "NS":
    case "CNAME":
    case "PTR":
      return data;
    case "MX": {
      const [priority, ...exchange] = data.split(/\s+/);
      return { exchange: exchange.join(" "), priority: Number(priority) };
    }
    case "TXT": return quoted(answerData(answer));
    case "SRV": {
      const [priority, weight, port, ...target] = data.split(/\s+/);
      return { name: target.join(" "), port: Number(port), priority: Number(priority), weight: Number(weight) };
    }
    case "SOA": {
      const [nsname, hostmaster, serial, refresh, retry, expire, minttl] = data.split(/\s+/);
      return { nsname, hostmaster, serial: Number(serial), refresh: Number(refresh), retry: Number(retry), expire: Number(expire), minttl: Number(minttl) };
    }
    case "CAA": {
      const match = /^(\d+)\s+(\S+)\s+"?([^\"]*)"?$/.exec(answerData(answer));
      return match ? { critical: Number(match[1]), issue: match[2], value: match[3] } : { critical: 0, issue: "", value: data };
    }
    case "NAPTR": {
      const fields = answerData(answer).match(/^(\d+)\s+(\d+)\s+"([^"]*)"\s+"([^"]*)"\s+"([^"]*)"\s+(\S+)$/);
      return fields
        ? { order: Number(fields[1]), preference: Number(fields[2]), flags: fields[3], service: fields[4], regexp: fields[5], replacement: fields[6].replace(/\.$/, "") }
        : { order: 0, preference: 0, flags: "", service: "", regexp: "", replacement: data };
    }
    default: return data;
  }
}

function answerWithTtl(answer: unknown): Record<string, unknown> {
  const value = answerValue(answer);
  return typeof value === "string" ? { address: value, ttl: answerTtl(answer) } : { ...value, ttl: answerTtl(answer) };
}

async function dohQuery(name: unknown, type: unknown): Promise<unknown[]> {
  const hostname = validateName(name);
  const recordType = queryType(type);
  const response = await fetch(`${dohEndpoint}?name=${encodeURIComponent(hostname)}&type=${recordType}`, {
    headers: { accept: "application/dns-json" },
  });
  if (!response.ok) throw dnsError(hostname, "SERVFAIL");
  const result: unknown = await response.json();
  const status = propertyOf(result, "Status");
  const answers = propertyOf(result, "Answer");
  if (status !== 0 || !Array.isArray(answers) || answers.length === 0) {
    throw dnsError(hostname, status === 3 ? "ENOTFOUND" : "ENODATA");
  }
  return answers.filter((answer: unknown) => propertyOf(answer, "type") === typeNumbers.get(recordType));
}

function resolvePromise(name: unknown, type: string, options: unknown = {}): Promise<unknown[]> {
  return dohQuery(name, type).then(answers => {
    if (type === "TXT") return answers.map(answer => answerValue(answer));
    return answers.map(answer => propertyOf(options, "ttl") ? answerWithTtl(answer) : answerValue(answer));
  });
}

function queryAllTypes(name: unknown): Promise<unknown[][]> {
  return Promise.all([...typeNumbers.keys()].map(type => dohQuery(name, type).catch(() => [])));
}

function anyRecords(answers: unknown[][]): Record<string, unknown>[] {
  return answers.flat().map(answer => ({ address: answerValue(answer), type: typeName(answer), ttl: answerTtl(answer) }));
}

function lastCallback(args: unknown[]): AnyFunction {
  const callback = args.at(-1);
  if (!isFunction(callback)) throw new TypeError("callback must be a function");
  return callback;
}

function callbackArgs(args: unknown[]): { name: unknown; options: unknown; callback: AnyFunction } {
  const callback = lastCallback(args);
  const options = args[1] !== null && typeof args[1] === "object" ? args[1] : {};
  return { name: args[0], options, callback };
}

function callbackResolve(type: string, args: unknown[]): void {
  const { name, options, callback } = callbackArgs(args);
  resolvePromise(name, type, options).then(value => callback(null, value), error => callback(error));
}

function unsupportedCallback(name: string, args: unknown[]): void {
  const callback = lastCallback(args);
  queueMicrotask(() => callback(unsupported(name)));
}

export function lookup(...args: unknown[]): void { unsupportedCallback("lookup", args); }
export function lookupService(...args: unknown[]): void { unsupportedCallback("lookupService", args); }
export function resolve(...args: unknown[]): void { unsupportedCallback("resolve", args); }
export function resolve4(...args: unknown[]): void { callbackResolve("A", args); }
export function resolve6(...args: unknown[]): void { callbackResolve("AAAA", args); }
export function resolveAny(...args: unknown[]): void {
  const { name, callback } = callbackArgs(args);
  queryAllTypes(name).then(answers => { callback(null, anyRecords(answers)); }, error => callback(error));
}
export function resolveCaa(...args: unknown[]): void { callbackResolve("CAA", args); }
export function resolveCname(...args: unknown[]): void { callbackResolve("CNAME", args); }
export function resolveMx(...args: unknown[]): void { callbackResolve("MX", args); }
export function resolveNaptr(...args: unknown[]): void { callbackResolve("NAPTR", args); }
export function resolveNs(...args: unknown[]): void { callbackResolve("NS", args); }
export function resolvePtr(...args: unknown[]): void { callbackResolve("PTR", args); }
export function resolveSoa(...args: unknown[]): void { callbackResolve("SOA", args); }
export function resolveSrv(...args: unknown[]): void { callbackResolve("SRV", args); }
export function resolveTxt(...args: unknown[]): void { callbackResolve("TXT", args); }
export function reverse(...args: unknown[]): void {
  const { name, callback } = callbackArgs(args);
  const labels = validateName(name).split(".").reverse().join(".") + ".in-addr.arpa";
  resolvePromise(labels, "PTR").then(value => callback(null, value), error => callback(error));
}
export function getDefaultResultOrder(): string { return "verbatim"; }
export function setDefaultResultOrder(): void {}
export function getServers(): string[] { return []; }
export function setServers(): void {}

export class Resolver {
  lookup(...args: unknown[]): void { lookup(...args); }
  lookupService(...args: unknown[]): void { lookupService(...args); }
  resolve(...args: unknown[]): void { resolve(...args); }
  resolve4(...args: unknown[]): void { resolve4(...args); }
  resolve6(...args: unknown[]): void { resolve6(...args); }
  resolveAny(...args: unknown[]): void { resolveAny(...args); }
  resolveCaa(...args: unknown[]): void { resolveCaa(...args); }
  resolveCname(...args: unknown[]): void { resolveCname(...args); }
  resolveMx(...args: unknown[]): void { resolveMx(...args); }
  resolveNaptr(...args: unknown[]): void { resolveNaptr(...args); }
  resolveNs(...args: unknown[]): void { resolveNs(...args); }
  resolvePtr(...args: unknown[]): void { resolvePtr(...args); }
  resolveSoa(...args: unknown[]): void { resolveSoa(...args); }
  resolveSrv(...args: unknown[]): void { resolveSrv(...args); }
  resolveTxt(...args: unknown[]): void { resolveTxt(...args); }
  reverse(...args: unknown[]): void { reverse(...args); }
  getServers(): string[] { return getServers(); }
  setServers(): void { setServers(); }
}

export const promises = {
  ...codes,
  Resolver,
  getDefaultResultOrder,
  getServers,
  lookup: () => Promise.reject(unsupported("lookup")),
  lookupService: () => Promise.reject(unsupported("lookupService")),
  resolve: () => Promise.reject(unsupported("resolve")),
  resolve4: (name: unknown, options: unknown) => resolvePromise(name, "A", options),
  resolve6: (name: unknown, options: unknown) => resolvePromise(name, "AAAA", options),
  resolveAny: (name: unknown) => queryAllTypes(name).then(answers => anyRecords(answers)),
  resolveCaa: (name: unknown, options: unknown) => resolvePromise(name, "CAA", options),
  resolveCname: (name: unknown, options: unknown) => resolvePromise(name, "CNAME", options),
  resolveMx: (name: unknown, options: unknown) => resolvePromise(name, "MX", options),
  resolveNaptr: (name: unknown, options: unknown) => resolvePromise(name, "NAPTR", options),
  resolveNs: (name: unknown, options: unknown) => resolvePromise(name, "NS", options),
  resolvePtr: (name: unknown, options: unknown) => resolvePromise(name, "PTR", options),
  resolveSoa: (name: unknown, options: unknown) => resolvePromise(name, "SOA", options),
  resolveSrv: (name: unknown, options: unknown) => resolvePromise(name, "SRV", options),
  resolveTxt: (name: unknown, options: unknown) => resolvePromise(name, "TXT", options),
  reverse: (name: unknown) => resolvePromise(`${validateName(name).split(".").reverse().join(".")}.in-addr.arpa`, "PTR"),
  setDefaultResultOrder,
  setServers,
};

export default {
  ...codes,
  Resolver, getDefaultResultOrder, getServers, lookup, lookupService, resolve, resolve4, resolve6, resolveAny,
  resolveCaa, resolveCname, resolveMx, resolveNaptr, resolveNs, resolvePtr, resolveSoa, resolveSrv, resolveTxt,
  reverse, setDefaultResultOrder, setServers, promises,
};
