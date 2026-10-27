import { promises } from "./dns.mjs";

export * from "./dns-codes.mjs";
export { Resolver, getDefaultResultOrder, getServers, setDefaultResultOrder, setServers } from "./dns.mjs";

export const {
  lookup, lookupService, resolve, resolve4, resolve6, resolveAny, resolveCaa, resolveCname, resolveMx, resolveNaptr,
  resolveNs, resolvePtr, resolveSoa, resolveSrv, resolveTxt, reverse,
} = promises;

const flags = ["ADDRCONFIG", "ALL", "V4MAPPED"];
export default Object.fromEntries(Object.entries(promises).filter(([name]) => !flags.includes(name)));
