import { createTracing } from "../builtins/tracing.mjs";

// Spans on the device are not traced.
const tracing = createTracing();

export function withSpan(name, callback) {
  return tracing.enterSpan(name, callback);
}
