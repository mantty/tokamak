import { createTracing } from "../builtins/tracing.ts";

// Spans on the device are not traced.
const tracing = createTracing();

export function withSpan(name: string, callback: (span: unknown) => unknown): unknown {
  return tracing.enterSpan(name, callback);
}
