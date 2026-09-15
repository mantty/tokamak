import { TextDecoder } from "../streams/text.mjs";
import { Blob, Buffer } from "./buffer.mjs";

function nodeIterator(stream) {
  const queue = [];
  let ended = false;
  let failure;
  let wake;

  const remove = (name, listener) => {
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
  const onData = chunk => { queue.push(chunk); notify(); };
  const onEnd = () => { ended = true; notify(); };
  const onError = error => { failure = error; ended = true; notify(); };
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
      while (queue.length === 0 && !ended) await new Promise(resolve => { wake = resolve; });
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

function webIterator(stream) {
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

function iterable(stream) {
  if (stream !== null && stream !== undefined) {
    if (typeof stream.getReader === "function") return webIterator(stream);
    if (typeof stream[Symbol.asyncIterator] === "function") return stream;
    if (typeof stream.on === "function") return nodeIterator(stream);
  }
  return stream;
}

async function collect(stream) {
  const source = iterable(stream);
  if (source != null
    && typeof source[Symbol.asyncIterator] !== "function"
    && typeof source[Symbol.iterator] !== "function") return [];
  const chunks = [];
  for await (const chunk of source) chunks.push(chunk);
  return chunks;
}

export async function text(stream) {
  const decoder = new TextDecoder();
  let value = "";
  for await (const chunk of iterable(stream)) {
    value += typeof chunk === "string" ? chunk : decoder.decode(chunk, { stream: true });
  }
  return value + decoder.decode();
}

export async function blob(stream) {
  return new Blob(await collect(stream));
}

export async function arrayBuffer(stream) {
  return (await blob(stream)).arrayBuffer();
}

export async function buffer(stream) {
  return Buffer.from(await arrayBuffer(stream));
}

export async function json(stream) {
  return JSON.parse(await text(stream));
}

const consumers = { arrayBuffer, blob, buffer, json, text };
export default consumers;
