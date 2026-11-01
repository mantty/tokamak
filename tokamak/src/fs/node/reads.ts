// workerd's checks and targets of node:fs reads, which the host's reads use.
import { Buffer } from "../../node/buffer.ts";

// Whether the callback `fs.read` range `offset`, `length` falls outside a
// buffer of `size` bytes, by workerd's check. `length` is still the value the
// caller passed, which JavaScript's `<` and `+` coerce as they do in workerd.
export function outsideBuffer(offset: number, length: unknown, size: number) {
  const coerced = length as number;
  return offset < 0 || coerced < 0 || (coerced !== 0 && offset + coerced > size);
}

// workerd's `readSync` target, which reads the properties a Worker may define
// on the view.
export function readTarget(view: ArrayBufferView, offset: number, length: number) {
  return Buffer.from(view.buffer, view.byteOffset + offset, length);
}
