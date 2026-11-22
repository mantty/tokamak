import assert from "node:assert/strict";
import { test } from "node:test";

import { eventCodec, setEventCodec } from "../src/events.js";

void test("events travel as JSON unless a plugin sets their handler's codec", () => {
  const json = eventCodec("_onQuiet");
  assert.deepEqual(json.decode(new TextEncoder().encode('{"a":1}')), { a: 1 });
  assert.deepEqual(json.encode({ a: 1 }), new TextEncoder().encode('{"a":1}'));

  const bytes = { decode: (payload: Uint8Array) => payload, encode: (reply: Uint8Array) => reply };
  setEventCodec("_onBinary", bytes);
  assert.equal(eventCodec("_onBinary"), bytes);
});
