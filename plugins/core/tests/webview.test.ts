import assert from "node:assert/strict";
import { test } from "node:test";
import { connectNative } from "../src/testing.js";

void test("WebView controls use the receiving invocation and preserve message data", async () => {
  const native = connectNative();
  const navigation = native.ctx.webview.navigate("/recipes?region=fr");
  assert.deepEqual(native.lastRequest(), {
    type: "call", plugin: "tokamak", method: "webview.navigate", arguments: "/recipes?region=fr",
  });
  native.respond({ value: null });
  await navigation;
  const current = native.ctx.webview.getCurrentPage();
  native.respond({ value: "https://app.tokamak.local/recipes?region=fr" });
  assert.equal(await current, "https://app.tokamak.local/recipes?region=fr");
  const message = native.ctx.webview.postMessage({ text: "</script>\u2028", count: 42 });
  assert.deepEqual(native.lastRequest().arguments, { text: "</script>\u2028", count: 42 });
  native.respond({ error: { name: "InvalidStateError", message: "No page is loaded" } });
  await assert.rejects(message, { name: "InvalidStateError", message: "No page is loaded" });
});
