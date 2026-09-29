import assert from "node:assert/strict";
import { afterEach, test } from "node:test";

import { acceptsRuntimeCall } from "../src/worker.js";

const nodeEnv = process.env.NODE_ENV;

afterEach(() => {
  Reflect.deleteProperty(globalThis, "__tokamak_runtime_call");
  process.env.NODE_ENV = nodeEnv;
});

void test("accepts a call the runtime made", () => {
  process.env.NODE_ENV = "production";
  globalThis.__tokamak_runtime_call = true;

  assert.equal(acceptsRuntimeCall(), true);
});

void test("accepts any request in development", () => {
  process.env.NODE_ENV = "development";

  assert.equal(acceptsRuntimeCall(), true);
});

void test("refuses other requests", () => {
  process.env.NODE_ENV = "production";
  globalThis.__tokamak_runtime_call = false;

  assert.equal(acceptsRuntimeCall(), false);
});
