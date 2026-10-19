import assert from 'node:assert/strict';
import test from 'node:test';
import { takeStream, receiveStream } from '../../src/rpc/streams.mjs';

test('stream transfer moves ownership and only reads when the receiver demands bytes', async () => {
  let pulls = 0;
  const source = new ReadableStream({
    type: 'bytes',
    pull(controller) {
      pulls++;
      controller.enqueue(new Uint8Array([1, 2, 3]));
      controller.close();
    },
  });
  const received = receiveStream(takeStream(source));
  assert.equal(source.locked, true);
  assert.equal(pulls, 0);
  const reader = received.getReader();
  assert.deepEqual(await reader.read(), { done: false, value: new Uint8Array([1, 2, 3]) });
  assert.equal(pulls, 1);
  assert.equal((await reader.read()).done, true);
});

test('transport chunks remain bounded when the source supplies a large buffer', async () => {
  const source = new ReadableStream({
    type: 'bytes',
    start(controller) {
      controller.enqueue(new Uint8Array(200_000).fill(5));
      controller.close();
    },
  });
  let total = 0;
  for await (const value of receiveStream(takeStream(source))) {
    assert.ok(value.byteLength <= 64 * 1024);
    assert.ok(value.every(byte => byte === 5));
    total += value.byteLength;
  }
  assert.equal(total, 200_000);
});

test('receiver cancellation reaches the producer, including while a read is pending', async () => {
  let reason;
  const source = new ReadableStream({ type: 'bytes', cancel(value) { reason = value; } });
  const reader = receiveStream(takeStream(source)).getReader();
  const pending = reader.read();
  await reader.cancel('no longer needed');
  assert.equal(reason, 'no longer needed');
  assert.equal((await pending).done, true);
});

test('producer failures reach the receiving reader', async () => {
  const source = new ReadableStream({
    type: 'bytes',
    pull(controller) { controller.error(Object.assign(new Error('device lost'), { code: 'DISCONNECTED' })); },
  });
  await assert.rejects(receiveStream(takeStream(source)).getReader().read(), { message: 'device lost', code: 'DISCONNECTED' });
});

test('a closed stream settles a pending BYOB read', async () => {
  const source = new ReadableStream({ type: 'bytes', start(controller) { controller.close(); } });
  const reader = receiveStream(takeStream(source)).getReader({ mode: 'byob' });
  assert.equal((await reader.read(new Uint8Array(8))).done, true);
});

test('ordinary streams that produce byte arrays use the same bounded transfer contract', async () => {
  const source = new ReadableStream({
    start(controller) {
      controller.enqueue(new Uint8Array(150_000).fill(7));
      controller.close();
    },
  });
  let total = 0;
  for await (const chunk of receiveStream(takeStream(source))) {
    assert.ok(chunk.byteLength <= 64 * 1024);
    assert.ok(chunk.every(value => value === 7));
    total += chunk.byteLength;
  }
  assert.equal(total, 150_000);
});

test('non-byte chunks fail the receiving read and cancel the producer', async () => {
  let cancelled;
  const source = new ReadableStream({
    start(controller) { controller.enqueue({ location: 'not bytes' }); },
    cancel(reason) { cancelled = reason; },
  });
  await assert.rejects(receiveStream(takeStream(source)).getReader().read(), { name: 'DataCloneError' });
  assert.equal(cancelled.name, 'DataCloneError');
});

test('an invalid chunk rejects even when producer cleanup fails or never completes', { timeout: 1000 }, async () => {
  for (const cancel of [() => Promise.reject(new Error('cleanup failed')), () => new Promise(() => {})]) {
    const source = new ReadableStream({
      start(controller) { controller.enqueue('not bytes'); },
      cancel,
    });
    await assert.rejects(receiveStream(takeStream(source)).getReader().read(), { name: 'DataCloneError' });
  }
});
