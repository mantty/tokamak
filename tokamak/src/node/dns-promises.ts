import { promises } from "./dns.ts";

export * from "./dns-codes.ts";
export { Resolver, getDefaultResultOrder, getServers, setDefaultResultOrder, setServers } from "./dns.ts";

export const {
  lookup, lookupService, resolve, resolve4, resolve6, resolveAny, resolveCaa, resolveCname, resolveMx, resolveNaptr,
  resolveNs, resolvePtr, resolveSoa, resolveSrv, resolveTxt, reverse,
} = promises;

const flags = ["ADDRCONFIG", "ALL", "V4MAPPED"];
export default Object.fromEntries(Object.entries(promises).filter(([name]) => !flags.includes(name)));
