import { encodeJson } from "./codec.js";

type Source<T> = (next: (value: T) => void, error: (error: unknown) => void, close: () => void) => () => void;
const maxBufferedBytes = 1024 * 1024;

/** Adapts a push source with bounded buffering and explicit cancellation. */
export function byteStream(subscribe: Source<Uint8Array>): ReadableStream<Uint8Array> {
  let stop: (() => void) | undefined;
  let active = true;
  return new ReadableStream<Uint8Array>({
    start(controller) {
      const finish = (error?: unknown) => {
        if (!active) return;
        active = false;
        if (error === undefined) controller.close();
        else controller.error(error);
        stop?.();
      };
      try {
        stop = subscribe((value) => {
          if (!active) return;
          if (value.byteLength > (controller.desiredSize ?? 0)) {
            finish(new DOMException("The stream consumer is too slow", "QuotaExceededError"));
            return;
          }
          controller.enqueue(value);
        }, finish, () => { finish(); });
        if (!active) stop();
      } catch (error) { finish(error); }
    },
    cancel() { active = false; stop?.(); },
  }, { highWaterMark: maxBufferedBytes, size: (value) => value.byteLength });
}

/** Encodes a typed push source as newline-delimited JSON. Errors terminate the stream. */
export function valueStream<T>(subscribe: Source<T>): ReadableStream<Uint8Array> {
  return byteStream((next, error, close) => subscribe((value) => {
    try {
      const payload = encodeJson(value);
      const line = new Uint8Array(payload.byteLength + 1);
      line.set(payload);
      line[payload.byteLength] = 10;
      next(line);
    } catch (failure) { error(failure); }
  }, error, close));
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
