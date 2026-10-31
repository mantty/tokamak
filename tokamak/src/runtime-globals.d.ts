// The globals the runtime's modules define for each other and for the Rust
// host, which reads them by name.
declare global {
  // The execution context passed to the Worker's handlers, which waits for
  // any value.
  interface TokamakContext extends ExecutionContext {
    waitUntil(value: unknown): void;
  }

  var __tokamak_context: TokamakContext;
  var __tokamak_env: Record<PropertyKey, unknown>;
  var __tokamak_exports: Record<PropertyKey, unknown> | undefined;
  var __tokamak_drain_wait_until: () => Promise<void>;
}

export {};
