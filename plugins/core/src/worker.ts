declare const process:
  | { readonly env: { readonly NODE_ENV?: string; readonly TOKAMAK_RUNTIME?: string } }
  | undefined;

/**
 * Whether the Worker serves the tokamak runtime's calls to endpoints under `/tokamak/`: in a
 * tokamak app, where the runtime sets `TOKAMAK_RUNTIME`, and in development. Those endpoints
 * refuse requests otherwise.
 */
export function acceptsRuntimeCalls(): boolean {
  // One expression: Rollup 4.63.0 drops callers' checks when this is an early return followed
  // by a return of `typeof process !== "undefined" && false`.
  return (
    typeof process !== "undefined" &&
    (process.env.NODE_ENV === "development" || process.env.TOKAMAK_RUNTIME === "true")
  );
}
