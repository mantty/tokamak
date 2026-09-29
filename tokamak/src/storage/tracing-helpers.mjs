import { createTracing } from "../builtins/tracing.mjs";

// Spans on the device are not traced.
const tracing = createTracing();

export function withSpan(name, callback) {
  return callback(tracing.startSpan(name));
}

export const startActiveSpan = withSpan;
