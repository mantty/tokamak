export const env = globalThis.__tokamak_env;
export function waitUntil(promise) { globalThis.__tokamak_context.waitUntil(promise); }
