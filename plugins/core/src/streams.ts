import { encodeJson } from "./codec.js";
import type { Overflow } from "./context.js";

type Source<T> = (next: (value: T) => void, error: (error: unknown) => void, close: () => void) => () => void;
const maxBufferedBytes = 1024 * 1024;

/**
 * Adapts a push source with explicit cancellation, buffering up to 1 MiB for a slow reader. Past
 * that, `overflow` ends the stream with `QuotaExceededError` or drops the oldest buffered values.
 * An error ends the stream at once; closing it delivers what is buffered first.
 */
export function byteStream(subscribe: Source<Uint8Array>, overflow: Overflow = "error"): ReadableStream<Uint8Array> {
  const buffered: Uint8Array[] = [];
  let bytes = 0;
  let end: { error?: unknown } | undefined;
  let stop = () => {};
  let wake = () => {};
  const finish = (error?: unknown) => {
    if (end) return;
    end = { error };
    if (error !== undefined) buffered.length = 0;
    stop();
    wake();
  };
  const push = (value: Uint8Array) => {
    if (end) return;
    buffered.push(value);
    bytes += value.byteLength;
    while (bytes > maxBufferedBytes) {
      const oldest = overflow === "dropOldest" && buffered.length > 1 ? buffered.shift() : undefined;
      if (!oldest) {
        finish(new DOMException("The stream consumer is too slow", "QuotaExceededError"));
        return;
      }
      bytes -= oldest.byteLength;
    }
    wake();
  };
  return new ReadableStream<Uint8Array>({
    start() {
      try {
        stop = subscribe(push, finish, () => { finish(); });
        if (end) stop();
      } catch (error) { finish(error); }
    },
    async pull(controller) {
      while (!buffered.length && !end) await new Promise<void>((resolve) => { wake = resolve; });
      const value = buffered.shift();
      if (value) {
        bytes -= value.byteLength;
        controller.enqueue(value);
      } else if (end?.error === undefined) controller.close();
      else controller.error(end.error);
    },
    cancel() { finish(); },
  }, { highWaterMark: 0 });
}

/** Encodes a typed push source as newline-delimited JSON. Errors terminate the stream. */
export function valueStream<T>(subscribe: Source<T>, overflow: Overflow = "error"): ReadableStream<Uint8Array> {
  return byteStream((next, error, close) => subscribe((value) => {
    try {
      const payload = encodeJson(value);
      const line = new Uint8Array(payload.byteLength + 1);
      line.set(payload);
      line[payload.byteLength] = 10;
      next(line);
    } catch (failure) { error(failure); }
  }, error, close), overflow);
}

/** Decodes JSON values; ending iteration cancels the source. */
export async function* decodeValues<T>(source: ReadableStream<Uint8Array> | Promise<ReadableStream<Uint8Array>>, signal: AbortSignal = new AbortController().signal): AsyncGenerator<T, void, unknown> {
  const reader = (await source).getReader();
  const cancel = () => { void reader.cancel().catch(() => undefined); };
  if (signal.aborted) cancel();
  else signal.addEventListener("abort", cancel, { once: true });
  try {
    yield* readValues<T>(reader, signal);
  } finally {
    // An errored source rejects cancel with its original failure.
    await reader.cancel().catch(() => undefined);
    signal.removeEventListener("abort", cancel);
    reader.releaseLock();
  }
}

async function* readValues<T>(reader: ReadableStreamDefaultReader<Uint8Array>, signal: AbortSignal): AsyncGenerator<T, void, unknown> {
  const decoder = new TextDecoder("utf-8", { fatal: true, ignoreBOM: false });
  let pending = "";
  let complete = false;
  while (!complete) {
    const { value, done } = await reader.read();
    if (signal.aborted) return;
    complete = done;
    pending += decoder.decode(value, { stream: !done });
    let newline: number;
    while ((newline = pending.indexOf("\n")) !== -1) {
      // eslint-disable-next-line @typescript-eslint/no-unnecessary-condition -- yielding lets the caller abort.
      if (signal.aborted) return;
      const line = pending.slice(0, newline);
      pending = pending.slice(newline + 1);
      yield parseValue<T>(line);
    }
    if (pending.length > maxBufferedBytes) throw new RangeError("The stream value exceeds 1 MiB");
  }
  if (pending) yield parseValue<T>(pending);
}

// eslint-disable-next-line @typescript-eslint/no-unnecessary-type-parameters -- the consumer supplies the decoded type.
function parseValue<T>(line: string): T {
  if (line.length > maxBufferedBytes) throw new RangeError("The stream value exceeds 1 MiB");
  return JSON.parse(line) as T;
}
