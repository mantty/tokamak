import assert from 'node:assert/strict';
import test, { type TestContext } from 'node:test';
import { Channel } from '../../src/rpc/channel.ts';
import { packFrame } from '../../src/rpc/frames.ts';
import { field } from '../../src/rpc/values.ts';

type Socket = EventTarget & { readyState: number; send: (data: ArrayBuffer) => void; close: () => void };

function sockets() {
  const pair: Socket[] = [];
  for (const index of [0, 1]) {
    const socket: Socket = Object.assign(new EventTarget(), {
      readyState: 1,
      send: (data: ArrayBuffer): void => {
        if (socket.readyState !== 1) throw new Error('Socket is closed');
        const copy = structuredClone(data);
        queueMicrotask(() => pair[1 - index].dispatchEvent(new MessageEvent('message', { data: copy })));
      },
      close: (): void => {
        if (socket.readyState === 3) return;
        for (const item of pair) item.readyState = 3;
        queueMicrotask(() => { for (const item of pair) item.dispatchEvent(new Event('close')); });
      },
    });
    pair.push(socket);
  }
  return pair;
}

function channels(t: TestContext) {
  const pair = sockets();
  t.after(() => { pair[0].close(); });
  return pair.map(socket => new Channel(socket));
}

async function receivedStream(channel: Channel): Promise<ReadableStream> {
  const stream = await channel.receive;
  assert.ok(stream instanceof ReadableStream);
  return stream;
}

void test('a channel transfers structured graphs and binary data without sharing state', async t => {
  const [sender, receiver] = channels(t);
  const buffer = new Uint8Array([1, 2, 3]).buffer;
  const value = { buffer, view: new DataView(buffer, 1), map: new Map<object, Set<object>>(), date: new Date(), integer: 2n ** 80n };
  value.map.set(value, new Set([value]));
  await sender.send(value);
  const copy = await receiver.receive;
  assert.deepEqual(copy, value);
  assert.equal(copy.view.buffer, copy.buffer);
  assert.equal(copy.map.get(copy)?.has(copy), true);
  new Uint8Array(copy.buffer)[0] = 9;
  assert.equal(new Uint8Array(buffer)[0], 1);
  await receiver.send({ value: undefined, error: Object.assign(new Error('denied', { cause: new TypeError('reason') }), { code: 'DENIED' }) });
  const error = field(await sender.receive, 'error');
  assert.ok(error instanceof Error);
  assert.equal(Reflect.get(error, 'code'), 'DENIED');
  assert.ok(error.cause instanceof Error);
  assert.equal(error.cause.message, 'reason');
  sender.complete();
  receiver.complete();
  await Promise.all([sender.closed, receiver.closed]);
});

void test('streams cross in both directions with demand-driven bounded reads', async t => {
  const [client, server] = channels(t);
  let pulls = 0;
  const source = new ReadableStream({
    type: 'bytes',
    pull(controller) {
      pulls++;
      controller.enqueue(new Uint8Array(200_000).fill(7));
      controller.close();
    },
  });
  await client.send({ source });
  const request = field(await server.receive, 'source');
  assert.equal(source.locked, true);
  assert.equal(pulls, 0);
  await server.send({ result: request, same: request });
  server.complete();
  const response = await client.receive;
  const result = field(response, 'result');
  assert.equal(result, field(response, 'same'));
  assert.ok(result instanceof ReadableStream);
  assert.equal(pulls, 0);
  let total = 0;
  for await (const chunk of result) {
    assert.ok(chunk instanceof Uint8Array);
    assert.ok(chunk.byteLength <= 64 * 1024);
    assert.ok(chunk.every(value => value === 7));
    total += chunk.byteLength;
  }
  assert.equal(total, 200_000);
  assert.equal(pulls, 1);
  await Promise.all([client.closed, server.closed]);
});

void test('cancelling a received stream settles its pending read and awaits producer cleanup', async t => {
  const [sender, receiver] = channels(t);
  let cancelled: unknown;
  const cleanup: PromiseWithResolvers<void> = Promise.withResolvers();
  const source = new ReadableStream({
    type: 'bytes',
    cancel(reason) { cancelled = reason; return cleanup.promise; },
  });
  await sender.send(source);
  const reader = (await receivedStream(receiver)).getReader();
  const reading = reader.read();
  const cancellation = reader.cancel({ reason: 'finished' });
  assert.equal((await reading).done, true);
  await new Promise(resolve => setImmediate(resolve));
  assert.deepEqual(cancelled, { reason: 'finished' });
  let settled = false;
  void cancellation.then(() => { settled = true; });
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(settled, false);
  cleanup.resolve();
  await cancellation;
  sender.complete();
  receiver.complete();
  await Promise.all([sender.closed, receiver.closed]);
});

void test('producer errors and cancellation errors reach the receiving stream', async t => {
  for (const failure of ['read', 'cancel']) {
    const [sender, receiver] = channels(t);
    const error = Object.assign(new Error('device unavailable'), { code: 'OFFLINE' });
    await sender.send(new ReadableStream({
      type: 'bytes',
      pull(controller) { if (failure === 'read') controller.error(error); },
      cancel() { throw error; },
    }));
    const reader = (await receivedStream(receiver)).getReader();
    await assert.rejects(failure === 'read' ? reader.read() : reader.cancel(), { message: error.message, code: error.code });
  }
});

void test('invalid stream chunks fail promptly even when producer cleanup hangs', { timeout: 1000 }, async t => {
  const [sender, receiver] = channels(t);
  await sender.send(new ReadableStream({
    start(controller) { controller.enqueue({ invalid: true }); },
    cancel() { return new Promise(() => {}); },
  }));
  await assert.rejects((await receivedStream(receiver)).getReader().read(), { name: 'DataCloneError' });
});

void test('invalid values fail before transferring streams or delivering a message', async t => {
  const [sender, receiver] = channels(t);
  const source = new ReadableStream({ type: 'bytes' });
  await assert.rejects(sender.send({ source, callback() {} }), { name: 'DataCloneError' });
  assert.equal(source.locked, false);
  await sender.send('valid');
  assert.equal(await receiver.receive, 'valid');
});

void test('disconnect rejects pending work and cancels owned producers', async t => {
  const [sender, receiver] = channels(t);
  let reason: unknown;
  await sender.send(new ReadableStream({ type: 'bytes', cancel(value) { reason = value; } }));
  const reader = (await receivedStream(receiver)).getReader();
  const reading = assert.rejects(reader.read(), { name: 'AbortError' });
  const reply = assert.rejects(sender.receive, { name: 'AbortError' });
  receiver.close();
  await Promise.all([reading, reply, sender.closed, receiver.closed]);
  assert.ok(reason instanceof DOMException);
  assert.equal(reason.name, 'AbortError');
});

void test('a disconnected stream rejects a later read without waiting for a frame', async t => {
  const [sender, receiver] = channels(t);
  await sender.send(new ReadableStream({ type: 'bytes' }));
  const stream = await receivedStream(receiver);
  sender.close();
  await receiver.closed;
  await assert.rejects(stream.getReader().read(), { name: 'AbortError' });
});

void test('a call snapshots its arguments while waiting for the socket to connect', async t => {
  const pair = sockets();
  pair[0].readyState = 0;
  t.after(() => { pair[0].close(); });
  const [sender, receiver] = pair.map(socket => new Channel(socket));
  const bytes = new Uint8Array([1, 2, 3]);
  const sending = sender.send(bytes);
  bytes[0] = 9;
  pair[0].readyState = 1;
  pair[0].dispatchEvent(new Event('open'));
  await sending;
  assert.deepEqual(await receiver.receive, new Uint8Array([1, 2, 3]));
});

void test('simultaneous streams keep their reads and cancellation independent', async t => {
  const [sender, receiver] = channels(t);
  let cancelled: unknown;
  await sender.send({
    idle: new ReadableStream({ type: 'bytes', cancel: reason => { cancelled = reason; } }),
    finite: new ReadableStream({
      type: 'bytes',
      start(controller) { controller.enqueue(new Uint8Array([5])); controller.close(); },
    }),
  });
  const value = await receiver.receive;
  const idle = field(value, 'idle'), finite = field(value, 'finite');
  assert.ok(idle instanceof ReadableStream && finite instanceof ReadableStream);
  const first = idle.getReader(), second = finite.getReader();
  const pending = first.read();
  assert.deepEqual(await second.read(), { done: false, value: new Uint8Array([5]) });
  await first.cancel('finished');
  assert.equal((await pending).done, true);
  assert.equal(cancelled, 'finished');
  assert.equal((await second.read()).done, true);
  sender.complete();
  receiver.complete();
  await Promise.all([sender.closed, receiver.closed]);
});

void test('malformed frames close the channel and release its owned streams', async t => {
  const pair = sockets();
  t.after(() => { pair[0].close(); });
  const sender = new Channel(pair[0]);
  let cancelled: unknown;
  await sender.send(new ReadableStream({ type: 'bytes', cancel: reason => { cancelled = reason; } }));
  const response = assert.rejects(sender.receive);
  pair[1].send(packFrame({ type: 'value', streams: [], data: '{"root":["ref",9],"nodes":[]}' }, []));
  await Promise.all([response, sender.closed]);
  assert.ok(cancelled instanceof Error);
});

void test('disconnect rejects readers immediately and retains the producer until async cleanup settles', async t => {
  const [sender, receiver] = channels(t);
  const cleanup: PromiseWithResolvers<void> = Promise.withResolvers();
  t.after(() => { cleanup.resolve(); });
  let started = false, settled = false;
  await sender.send(new ReadableStream({
    type: 'bytes',
    cancel() { started = true; return cleanup.promise; },
  }));
  const reading = assert.rejects((await receivedStream(receiver)).getReader().read(), { name: 'AbortError' });
  void sender.closed.then(() => { settled = true; });
  receiver.close();
  await reading;
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(started, true);
  assert.equal(settled, false);
  cleanup.resolve();
  await sender.closed;
});

void test('cancellation crossing a final read settles without a connection error', async t => {
  const pair = sockets();
  t.after(() => { pair[0].close(); });
  const [sender, receiver] = pair.map(socket => new Channel(socket));
  await sender.send(new ReadableStream({ type: 'bytes', start(controller) { controller.close(); } }));
  const reader = (await receivedStream(receiver)).getReader();
  const send = pair[0].send;
  let cancellation: Promise<void> | undefined;
  pair[0].send = data => {
    send(data);
    cancellation ??= reader.cancel('finished');
  };
  sender.complete();
  assert.equal((await reader.read()).done, true);
  await cancellation;
  await Promise.all([sender.closed, receiver.closed]);
});

void test('cancellation crossing a terminal error preserves its transferred stream fields', async t => {
  const pair = sockets();
  t.after(() => { pair[0].close(); });
  let cancelled = false;
  const error = Object.assign(new Error('Stream failed'), {
    details: new ReadableStream({ type: 'bytes', cancel() { cancelled = true; } }),
  });
  const [sender, receiver] = pair.map(socket => new Channel(socket));
  await sender.send(new ReadableStream({ type: 'bytes', pull(controller) { controller.error(error); } }));
  const reader = (await receivedStream(receiver)).getReader();
  const send = pair[0].send;
  let cancellation: Promise<unknown> | undefined;
  pair[0].send = data => {
    send(data);
    cancellation ??= reader.cancel().catch((error: unknown) => error);
  };
  sender.complete();
  assert.equal((await reader.read()).done, true);
  const failure = await cancellation;
  assert.ok(failure instanceof Error);
  assert.equal(failure.message, 'Stream failed');
  const details: unknown = Reflect.get(failure, 'details');
  assert.ok(details instanceof ReadableStream);
  await details.cancel();
  await Promise.all([sender.closed, receiver.closed]);
  assert.equal(cancelled, true);
});
