import { promises } from "./dns.mjs";

export { constants, Resolver, getDefaultResultOrder, getServers, setDefaultResultOrder, setServers } from "./dns.mjs";

export const {
  lookup, lookupService, resolve, resolve4, resolve6, resolveAny, resolveCaa, resolveCname, resolveMx, resolveNaptr,
  resolveNs, resolvePtr, resolveSoa, resolveSrv, resolveTxt, reverse,
} = promises;

export default promises;
