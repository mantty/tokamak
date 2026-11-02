import { hasMethod, isAsyncIterable, isIterable, type AnyFunction } from "../globals/objects.ts";
import { TextDecoder } from "../streams/text.ts";
import { Blob, Buffer } from "./buffer.ts";

// A Node stream, which its `on` method identifies.
interface NodeStream {
  on(name: string, listener: AnyFunction): unknown;
  removeListener?(name: string, listener: AnyFunction): unknown;
}

function nodeIterator(stream: NodeStream): AsyncIterableIterator<unknown> {
  const queue: unknown[] = [];
  let ended = false;
  let failure: unknown;
  let wake: (() => void) | undefined;

  const remove = (name: string, listener: AnyFunction) => {
    stream.removeListener?.(name, listener);
  };
  const cleanup = () => {
    remove("data", onData);
    remove("end", onEnd);
    remove("error", onError);
    remove("close", onClose);
  };
  const notify = () => {
    const resolve = wake;
    wake = undefined;
    resolve?.();
  };
  const onData = (chunk: unknown) => { queue.push(chunk); notify(); };
  const onEnd = () => { ended = true; notify(); };
  const onError = (error: unknown) => { failure = error; ended = true; notify(); };
  const onClose = () => {
    if (!ended) {
      failure = Object.assign(new Error("Premature close"), { code: "ERR_STREAM_PREMATURE_CLOSE" });
      ended = true;
      notify();
    }
  };

  stream.on("error", onError);
  stream.on("end", onEnd);
  stream.on("close", onClose);
  stream.on("data", onData);

  return {
    async next() {
      while (queue.length === 0 && !ended) await new Promise<void>(resolve => { wake = resolve; });
      if (queue.length > 0) return { done: false, value: queue.shift() };
      cleanup();
      if (failure) throw failure;
      return { done: true, value: undefined };
    },
    async return() {
      cleanup();
      return { done: true, value: undefined };
    },
    [Symbol.asyncIterator]() { return this; },
  };
}

// Whether `value` is a ReadableStream, which its `getReader` method identifies.
function isReadableStream(value: unknown): value is ReadableStream<unknown> {
  return hasMethod(value, "getReader");
}

function webIterator(stream: ReadableStream<unknown>): AsyncIterable<unknown> {
  return {
    async *[Symbol.asyncIterator]() {
      const reader = stream.getReader();
      try {
        while (true) {
          const result = await reader.read();
          if (result.done) return;
          yield result.value;
        }
      } finally {
        try { await reader.cancel(); }
        finally { reader.releaseLock(); }
      }
    },
  };
}

// The chunks of `stream`, undefined when it has none to iterate.
function chunks(stream: unknown): AsyncIterable<unknown> | Iterable<unknown> | undefined {
  if (stream === null || stream === undefined) return undefined;
  if (isReadableStream(stream)) return webIterator(stream);
  if (isAsyncIterable(stream)) return stream;
  if (hasMethod(stream, "on")) return nodeIterator(stream);
  if (isIterable(stream)) return stream;
  return undefined;
}

function notIterable(stream: unknown): TypeError {
  return new TypeError(`${String(stream)} is not async iterable`);
}

async function collect(stream: unknown): Promise<unknown[]> {
  const source = chunks(stream);
  if (source === undefined) {
    if (stream === null || stream === undefined) throw notIterable(stream);
    return [];
  }
  const values: unknown[] = [];
  for await (const chunk of source) values.push(chunk);
  return values;
}

export async function text(stream: unknown): Promise<string> {
  const source = chunks(stream);
  if (source === undefined) throw notIterable(stream);
  const decoder = new TextDecoder();
  let value = "";
  for await (const chunk of source) {
    value += typeof chunk === "string" ? chunk : decoder.decode(chunk, { stream: true });
  }
  return value + decoder.decode();
}

export async function blob(stream: unknown): Promise<Blob> {
  return new Blob(await collect(stream));
}

export async function arrayBuffer(stream: unknown): Promise<ArrayBuffer> {
  return (await blob(stream)).arrayBuffer();
}

export async function buffer(stream: unknown): Promise<Buffer> {
  return Buffer.from(await arrayBuffer(stream));
}

export async function json(stream: unknown): Promise<unknown> {
  return JSON.parse(await text(stream));
}

const consumers = { arrayBuffer, blob, buffer, json, text };
export default consumers;
