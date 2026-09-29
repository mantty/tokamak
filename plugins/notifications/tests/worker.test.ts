import assert from "node:assert/strict";
import { beforeEach, test } from "node:test";

import { handlePushRequest } from "../worker/index.js";

const message = { id: "m1", title: null, body: null, data: { itemId: "42" } };

beforeEach(() => {
  process.env.NODE_ENV = "production";
  Reflect.deleteProperty(process.env, "TOKAMAK_RUNTIME");
});

function push(): Request {
  return new Request("https://app.tokamak.local/tokamak/push", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(message),
  });
}

void test("runs the handler with the message and responds with its notification", async () => {
  process.env.TOKAMAK_RUNTIME = "true";

  const response = await handlePushRequest<{ itemId: string }>(push(), (received) => {
    assert.deepEqual(received, message);
    return { id: `item-${received.data.itemId}`, title: "New item" };
  });

  assert.equal(response.status, 200);
  assert.deepEqual(await response.json(), { id: "item-42", title: "New item" });
});

void test("responds null when the handler returns nothing", async () => {
  process.env.TOKAMAK_RUNTIME = "true";

  const response = await handlePushRequest(push(), async () => {});

  assert.equal(response.status, 200);
  assert.equal(await response.json(), null);
});

void test("responds 404 without running the handler outside a tokamak app", async () => {
  let ran = false;

  const response = await handlePushRequest(push(), () => {
    ran = true;
  });

  assert.equal(response.status, 404);
  assert.equal(ran, false);
});
