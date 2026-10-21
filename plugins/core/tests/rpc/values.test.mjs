import assert from 'node:assert/strict';
import test from 'node:test';
import { encode, decode } from '../../src/rpc/values.mjs';

function transfer(value) {
  const message = encode(value);
  return decode({ ...message, buffers: message.buffers.map(buffer => buffer.slice(0)) });
}

test('RPC values preserve data, cycles, and aliases without sharing mutable state', () => {
  const key = { id: 7 };
  const value = {
    missing: undefined, nil: null, flag: true, text: 'hello',
    numbers: [NaN, Infinity, -Infinity, -0, 1.25], large: 2n ** 90n,
    date: new Date('2026-10-19T12:00:00Z'), pattern: /hello/gi,
    map: new Map([[key, new Set([key])]]), key,
  };
  value.self = value;
  const copy = transfer(value);
  assert.deepEqual(copy, value);
  assert.notEqual(copy, value);
  assert.equal(copy.self, copy);
  assert.equal(copy.map.get(copy.key).has(copy.key), true);
  copy.key.id = 8;
  assert.equal(key.id, 7);
});

test('RPC buffers copy bytes and preserve views of the same buffer', () => {
  const buffer = new ArrayBuffer(16);
  new Uint8Array(buffer).set([1, 2, 3, 4]);
  const value = { buffer, bytes: new Uint8Array(buffer, 1, 2), view: new DataView(buffer, 2, 4) };
  const copy = transfer(value);
  assert.deepEqual(copy, value);
  assert.equal(copy.bytes.buffer, copy.buffer);
  assert.equal(copy.view.buffer, copy.buffer);
  copy.bytes[0] = 9;
  assert.equal(value.bytes[0], 2);
});

test('RPC errors retain their useful data and causes', () => {
  class DeviceError extends Error {}
  const cause = new TypeError('invalid input');
  const error = new DeviceError('device unavailable', { cause });
  error.name = 'DeviceError';
  Object.defineProperty(error, 'code', { value: 'DEVICE_OFFLINE' });
  error.details = { retry: true };
  const copy = transfer(error);
  assert.ok(copy instanceof Error);
  assert.equal(copy.name, 'DeviceError');
  assert.equal(copy.message, error.message);
  assert.equal(copy.code, error.code);
  assert.deepEqual(copy.details, error.details);
  assert.equal(copy.cause.name, 'TypeError');
  assert.equal(copy.cause.message, cause.message);
  const dom = transfer(new DOMException('permission denied', 'NotAllowedError'));
  assert.ok(dom instanceof DOMException);
  assert.equal(dom.name, 'NotAllowedError');
  assert.equal(dom.message, 'permission denied');
});

test('RPC arrays retain holes and records cannot mutate receiver prototypes', () => {
  const array = Array(4);
  array[2] = undefined;
  const value = JSON.parse('{"__proto__":{"polluted":true},"constructor":"data"}');
  const copy = transfer({ array, value });
  assert.equal(copy.array.length, 4);
  assert.equal(0 in copy.array, false);
  assert.equal(2 in copy.array, true);
  assert.equal(Object.hasOwn(copy.value, '__proto__'), true);
  assert.equal(copy.value.polluted, undefined);
  assert.equal(copy.value.constructor, 'data');
});

test('RPC rejects values that contain executable behaviour or unsupported host objects', () => {
  class DeviceHandle { constructor() { this.id = 1; } }
  const unsupported = [
    () => {}, Symbol('id'), new DeviceHandle(), new WeakMap(), new WeakSet(),
    new SharedArrayBuffer(4), Promise.resolve(1), new Request('https://example.com'),
    new Response('value'), new WritableStream(), new TransformStream(),
  ];
  for (const value of unsupported) {
    assert.throws(() => encode({ nested: value }), { name: 'DataCloneError' });
  }
  const error = new Error('failed');
  error.callback = () => {};
  assert.throws(() => encode(error), { name: 'DataCloneError' });
});

test('RPC stream discovery does not transfer ownership before the whole value is accepted', () => {
  const stream = new ReadableStream({ type: 'bytes' });
  const message = encode({ stream, again: stream });
  assert.equal(stream.locked, false);
  assert.deepEqual(message.streams, [stream]);
  const received = new ReadableStream({ type: 'bytes' });
  const copy = decode({ ...message, streams: [received] });
  assert.equal(copy.stream, received);
  assert.equal(copy.again, received);
  assert.throws(() => encode({ stream, invalid: () => {} }), { name: 'DataCloneError' });
  assert.equal(stream.locked, false);
});

test('RPC requires unlocked readable streams', () => {
  const ordinary = new ReadableStream();
  assert.equal(encode(ordinary).streams[0], ordinary);
  const stream = new ReadableStream({ type: 'bytes' });
  const reader = stream.getReader();
  assert.throws(() => encode(stream), { name: 'DataCloneError' });
  reader.releaseLock();
  assert.equal(encode(stream).streams[0], stream);
});
