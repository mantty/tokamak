import assert from "node:assert/strict";
import { test } from "node:test";
import { encodeMessage, decodeMessage, messages } from "../src/native-protocol.mjs";

test("native routing preserves arbitrary bytes independently of completion metadata", () => {
  const header = { type: "result", id: 42, failed: false, done: true };
  const payload = new Uint8Array([0, 255, 0xc0, 0xaf, 4]);
  const encoded = encodeMessage(header, payload);
  const decoded = decodeMessage(encoded);
  assert.deepEqual(decoded, { header, payload });
  assert.equal(decoded.payload.buffer, encoded.buffer);
});

test("native streaming accepts split and combined messages and rejects truncated input", async () => {
  const first = encodeMessage({ type: "result", id: 1 }, new Uint8Array([10]));
  const last = encodeMessage({ type: "result", id: 2, done: true }, new Uint8Array([20]));
  async function* chunks() {
    yield first.subarray(0, 2);
    yield first.subarray(2, 9);
    yield new Uint8Array([...first.subarray(9), ...last]);
  }
  assert.deepEqual(await Array.fromAsync(messages(chunks())), [
    { header: { type: "result", id: 1 }, payload: new Uint8Array([10]) },
    { header: { type: "result", id: 2, done: true }, payload: new Uint8Array([20]) },
  ]);
  await assert.rejects(async () => Array.fromAsync(messages([first.subarray(0, -1)])), /Incomplete/);
});


test("invalid routing headers fail before dispatch", () => {
  for (const header of [null, [], "result", {}, { type: "result", id: -1 }, { type: "result", id: "1" }, { type: "result", failed: "false" }, { type: "event", name: 42 }]) {
    assert.throws(() => decodeMessage(encodeMessage(header)), /Invalid native routing header/);
  }
});
