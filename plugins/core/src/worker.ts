declare const process: { readonly env: { readonly NODE_ENV?: string } } | undefined;

declare global {
  var __tokamak_runtime_call: boolean | undefined;
}

/**
 * Whether the request the Worker is handling may be a call from the tokamak runtime: one the
 * runtime made itself, or any request in development. Endpoints under `/tokamak/` refuse
 * other requests.
 */
export function acceptsRuntimeCall(): boolean {
  // One expression: Rollup 4.63.0 drops callers' checks when this is an early return followed
  // by a return of `typeof process !== "undefined" && false`.
  return (
    globalThis.__tokamak_runtime_call === true ||
    (typeof process !== "undefined" && process.env.NODE_ENV === "development")
  );
}
