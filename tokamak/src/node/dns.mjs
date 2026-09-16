const errorNames = [
  "ADDRCONFIG", "ADDRGETNETWORKPARAMS", "ALL", "BADFAMILY", "BADFLAGS", "BADHINTS", "BADNAME", "BADQUERY", "BADRESP",
  "BADSTR", "CANCELLED", "CONNREFUSED", "DESTRUCTION", "EOF", "FILE", "FORMERR", "LOADIPHLPAPI", "NODATA", "NOMEM",
  "NONAME", "NOTFOUND", "NOTIMP", "NOTINITIALIZED", "REFUSED", "SERVFAIL", "TIMEOUT", "V4MAPPED",
];
export const constants = Object.fromEntries(errorNames.map(name => [name, 0]));
const promiseConstants = Object.fromEntries(
  Object.entries(constants).filter(([name]) => !["ADDRCONFIG", "ALL", "V4MAPPED"].includes(name)),
);

const dohEndpoint = "https://1.1.1.1/dns-query";
const typeNumbers = {
  A: 1, NS: 2, CNAME: 5, SOA: 6, PTR: 12, MX: 15, TXT: 16, AAAA: 28, SRV: 33, NAPTR: 35, CAA: 257,
};

function dnsError(name, code = "ENOTFOUND") {
  const error = new Error(`query ${name} ${code}`);
  error.code = code;
  error.hostname = name;
  error.syscall = "queryA";
  return error;
}

function unsupported(name) {
  const error = new Error(`node:dns ${name} is not implemented in the Workers runtime`);
  error.code = "ERR_METHOD_NOT_IMPLEMENTED";
  error.syscall = name;
  return error;
}

function queryType(type) {
  const value = String(type).toUpperCase();
  if (typeNumbers[value] === undefined) throw new TypeError(`Unknown DNS record type: ${type}`);
  return value;
}

function validateName(name) {
  if (typeof name !== "string" || name.length === 0) throw new TypeError("The hostname must be a non-empty string");
  return name;
}

function quoted(value) {
  const parts = [];
  const pattern = /"((?:\\.|[^"\\])*)"/g;
  let match;
  while ((match = pattern.exec(value))) parts.push(match[1].replace(/\\([\\"])/g, "$1"));
  return parts.length ? parts : [value];
}

function answerValue(answer) {
  const type = Object.entries(typeNumbers).find(([, number]) => number === answer.type)?.[0] ?? answer.type;
  const data = String(answer.data ?? "").replace(/\.$/, "");
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
    case "TXT": return quoted(String(answer.data ?? ""));
    case "SRV": {
      const [priority, weight, port, ...target] = data.split(/\s+/);
      return { name: target.join(" "), port: Number(port), priority: Number(priority), weight: Number(weight) };
    }
    case "SOA": {
      const [nsname, hostmaster, serial, refresh, retry, expire, minttl] = data.split(/\s+/);
      return { nsname, hostmaster, serial: Number(serial), refresh: Number(refresh), retry: Number(retry), expire: Number(expire), minttl: Number(minttl) };
    }
    case "CAA": {
      const match = /^(\d+)\s+(\S+)\s+"?([^\"]*)"?$/.exec(String(answer.data ?? ""));
      return match ? { critical: Number(match[1]), issue: match[2], value: match[3] } : { critical: 0, issue: "", value: data };
    }
    case "NAPTR": {
      const fields = String(answer.data ?? "").match(/^(\d+)\s+(\d+)\s+"([^"]*)"\s+"([^"]*)"\s+"([^"]*)"\s+(\S+)$/);
      return fields
        ? { order: Number(fields[1]), preference: Number(fields[2]), flags: fields[3], service: fields[4], regexp: fields[5], replacement: fields[6].replace(/\.$/, "") }
        : { order: 0, preference: 0, flags: "", service: "", regexp: "", replacement: data };
    }
    default: return data;
  }
}

function answerWithTtl(answer) {
  const value = answerValue(answer);
  return typeof value === "string" ? { address: value, ttl: Number(answer.TTL ?? 0) } : { ...value, ttl: Number(answer.TTL ?? 0) };
}

async function dohQuery(name, type) {
  const hostname = validateName(name);
  const recordType = queryType(type);
  const response = await fetch(`${dohEndpoint}?name=${encodeURIComponent(hostname)}&type=${recordType}`, {
    headers: { accept: "application/dns-json" },
  });
  if (!response.ok) throw dnsError(hostname, "SERVFAIL");
  const result = await response.json();
  if (result.Status !== 0 || !Array.isArray(result.Answer) || result.Answer.length === 0) {
    throw dnsError(hostname, result.Status === 3 ? "ENOTFOUND" : "ENODATA");
  }
  return result.Answer.filter(answer => answer.type === typeNumbers[recordType]);
}

export function resolvePromise(name, type, options = {}) {
  return dohQuery(name, type).then(answers => {
    if (type === "TXT") return answers.map(answer => answerValue(answer));
    if (type === "MX" || type === "SOA" || type === "SRV" || type === "NAPTR" || type === "CAA") {
      return answers.map(answer => options.ttl ? answerWithTtl(answer) : answerValue(answer));
    }
    return answers.map(answer => options.ttl ? answerWithTtl(answer) : answerValue(answer));
  });
}

function callbackArgs(args) {
  const callback = args.at(-1);
  if (typeof callback !== "function") throw new TypeError("callback must be a function");
  const options = args[1] !== null && typeof args[1] === "object" ? args[1] : {};
  return { name: args[0], options, callback };
}

function callbackResolve(type, args) {
  const { name, options, callback } = callbackArgs(args);
  resolvePromise(name, type, options).then(value => callback(null, value), error => callback(error));
}

function unsupportedCallback(name, args) {
  const callback = args.at(-1);
  if (typeof callback !== "function") throw new TypeError("callback must be a function");
  queueMicrotask(() => callback(unsupported(name)));
}

export function lookup(...args) { unsupportedCallback("lookup", args); }
export function lookupService(...args) { unsupportedCallback("lookupService", args); }
export function resolve(...args) { unsupportedCallback("resolve", args); }
export function resolve4(...args) { callbackResolve("A", args); }
export function resolve6(...args) { callbackResolve("AAAA", args); }
export function resolveAny(...args) {
  const { name, callback } = callbackArgs(args);
  Promise.all(Object.keys(typeNumbers).map(type => dohQuery(name, type).catch(() => []))).then(answers => {
    callback(null, answers.flat().map(answer => ({ address: answerValue(answer), type: Object.entries(typeNumbers).find(([, number]) => number === answer.type)?.[0], ttl: Number(answer.TTL ?? 0) })));
  }, error => callback(error));
}
export function resolveCaa(...args) { callbackResolve("CAA", args); }
export function resolveCname(...args) { callbackResolve("CNAME", args); }
export function resolveMx(...args) { callbackResolve("MX", args); }
export function resolveNaptr(...args) { callbackResolve("NAPTR", args); }
export function resolveNs(...args) { callbackResolve("NS", args); }
export function resolvePtr(...args) { callbackResolve("PTR", args); }
export function resolveSoa(...args) { callbackResolve("SOA", args); }
export function resolveSrv(...args) { callbackResolve("SRV", args); }
export function resolveTxt(...args) { callbackResolve("TXT", args); }
export function reverse(...args) {
  const { name, callback } = callbackArgs(args);
  const labels = validateName(name).split(".").reverse().join(".") + ".in-addr.arpa";
  resolvePromise(labels, "PTR").then(value => callback(null, value), error => callback(error));
}
export function getDefaultResultOrder() { return "verbatim"; }
export function setDefaultResultOrder() {}
export function getServers() { return []; }
export function setServers() {}

export class Resolver {
  lookup(...args) { return lookup(...args); }
  lookupService(...args) { return lookupService(...args); }
  resolve(...args) { return resolve(...args); }
  resolve4(...args) { return resolve4(...args); }
  resolve6(...args) { return resolve6(...args); }
  resolveAny(...args) { return resolveAny(...args); }
  resolveCaa(...args) { return resolveCaa(...args); }
  resolveCname(...args) { return resolveCname(...args); }
  resolveMx(...args) { return resolveMx(...args); }
  resolveNaptr(...args) { return resolveNaptr(...args); }
  resolveNs(...args) { return resolveNs(...args); }
  resolvePtr(...args) { return resolvePtr(...args); }
  resolveSoa(...args) { return resolveSoa(...args); }
  resolveSrv(...args) { return resolveSrv(...args); }
  resolveTxt(...args) { return resolveTxt(...args); }
  reverse(...args) { return reverse(...args); }
  getServers() { return getServers(); }
  setServers(...args) { return setServers(...args); }
}

export const promises = {
  ...promiseConstants,
  Resolver,
  getDefaultResultOrder,
  getServers,
  lookup: () => Promise.reject(unsupported("lookup")),
  lookupService: () => Promise.reject(unsupported("lookupService")),
  resolve: () => Promise.reject(unsupported("resolve")),
  resolve4: (name, options) => resolvePromise(name, "A", options),
  resolve6: (name, options) => resolvePromise(name, "AAAA", options),
  resolveAny: name => Promise.all(Object.keys(typeNumbers).map(type => dohQuery(name, type).catch(() => []))).then(answers => answers.flat().map(answer => ({ address: answerValue(answer), type: Object.entries(typeNumbers).find(([, number]) => number === answer.type)?.[0], ttl: Number(answer.TTL ?? 0) }))),
  resolveCaa: (name, options) => resolvePromise(name, "CAA", options),
  resolveCname: (name, options) => resolvePromise(name, "CNAME", options),
  resolveMx: (name, options) => resolvePromise(name, "MX", options),
  resolveNaptr: (name, options) => resolvePromise(name, "NAPTR", options),
  resolveNs: (name, options) => resolvePromise(name, "NS", options),
  resolvePtr: (name, options) => resolvePromise(name, "PTR", options),
  resolveSoa: (name, options) => resolvePromise(name, "SOA", options),
  resolveSrv: (name, options) => resolvePromise(name, "SRV", options),
  resolveTxt: (name, options) => resolvePromise(name, "TXT", options),
  reverse: name => resolvePromise(`${validateName(name).split(".").reverse().join(".")}.in-addr.arpa`, "PTR"),
  setDefaultResultOrder,
  setServers,
};

export default {
  ...constants,
  Resolver, getDefaultResultOrder, getServers, lookup, lookupService, resolve, resolve4, resolve6, resolveAny,
  resolveCaa, resolveCname, resolveMx, resolveNaptr, resolveNs, resolvePtr, resolveSoa, resolveSrv, resolveTxt,
  reverse, setDefaultResultOrder, setServers, promises,
};
