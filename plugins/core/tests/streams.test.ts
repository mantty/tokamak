import assert from "node:assert/strict";
import { test } from "node:test";
import { byteStream, decodeValues, valueStream } from "../src/streams.js";

void test("decodes fragmented UTF-8 and multiple values in one chunk", async () => {
  const bytes = new TextEncoder().encode('"🍎"\n{"name":"line\\nbreak"}\n');
  const stream = new ReadableStream<Uint8Array>({ start(controller) {
    controller.enqueue(bytes.slice(0, 3));
    controller.enqueue(bytes.slice(3));
    controller.close();
  } });
  const result: unknown[] = [];
  for await (const value of decodeValues(stream)) result.push(value);
  assert.deepEqual(result, ["🍎", { name: "line\nbreak" }]);
});

void test("aborting a partial value cancels its source without decoding an incomplete record", async () => {
  const abort = new AbortController();
  let cancelled = 0;
  const values = decodeValues(new ReadableStream<Uint8Array>({
    start(controller) { controller.enqueue(new TextEncoder().encode('{"part":')); },
    cancel() { cancelled++; },
  }), abort.signal);
  const reading = values.next();
  await Promise.resolve();
  await Promise.resolve();
  abort.abort();
  assert.deepEqual(await reading, { value: undefined, done: true });
  assert.equal(cancelled, 1);
});

void test("aborting after a value discards other buffered records", async () => {
  const abort = new AbortController();
  const values = decodeValues(new ReadableStream<Uint8Array>({ start(controller) {
    controller.enqueue(new TextEncoder().encode('1\n2\n'));
  } }), abort.signal);
  assert.deepEqual(await values.next(), { value: 1, done: false });
  abort.abort();
  assert.equal((await values.next()).done, true);
});

void test("an overflowing push source is cancelled exactly once", async () => {
  let cancelled = 0;
  const stream = byteStream(next => {
    next(new Uint8Array(1024 * 1024 + 1));
    return () => { cancelled++; };
  });
  await assert.rejects(stream.getReader().read(), { name: "QuotaExceededError" });
  assert.equal(cancelled, 1);
});

void test("a synchronously completed value source preserves its final value", async () => {
  let cancelled = 0;
  const stream = valueStream<string>((next, _error, close) => {
    next("last");
    close();
    return () => { cancelled++; };
  });
  const values = decodeValues<string>(stream);
  assert.deepEqual(await values.next(), { value: "last", done: false });
  assert.equal((await values.next()).done, true);
  assert.equal(cancelled, 1);
});

void test("a drop-oldest source keeps the newest values within its buffer", async () => {
  const megabyte = 1024 * 1024;
  const stream = byteStream(next => {
    for (const marker of [1, 2, 3]) next(new Uint8Array(megabyte / 2).fill(marker));
    return () => undefined;
  }, "dropOldest");
  const reader = stream.getReader();
  assert.equal((await reader.read()).value?.[0], 2);
  assert.equal((await reader.read()).value?.[0], 3);
});

void test("closing delivers buffered values while an error discards them", async () => {
  const closed = byteStream((next, _error, close) => {
    next(new Uint8Array([1]));
    close();
    return () => undefined;
  });
  const reader = closed.getReader();
  assert.deepEqual((await reader.read()).value, new Uint8Array([1]));
  assert.equal((await reader.read()).done, true);

  const failed = byteStream((next, error) => {
    next(new Uint8Array([1]));
    error(new DOMException("Denied", "NotAllowedError"));
    return () => undefined;
  });
  await assert.rejects(failed.getReader().read(), { name: "NotAllowedError" });
});

void test("a source that ends while it subscribes is stopped, and one that throws rejects", async () => {
  let stopped = 0;
  const ended = byteStream((_next, _error, close) => {
    close();
    return () => { stopped++; };
  });
  assert.equal((await ended.getReader().read()).done, true);
  assert.equal(stopped, 1);

  const failed = byteStream(() => { throw new DOMException("Denied", "NotAllowedError"); });
  await assert.rejects(failed.getReader().read(), { name: "NotAllowedError" });
});
