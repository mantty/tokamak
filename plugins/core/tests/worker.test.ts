import assert from "node:assert/strict";
import { beforeEach, test } from "node:test";

import { acceptsRuntimeCalls } from "../src/worker.js";

beforeEach(() => {
  process.env.NODE_ENV = "production";
  Reflect.deleteProperty(process.env, "TOKAMAK_RUNTIME");
});

void test("accepts calls in a tokamak app", () => {
  process.env.TOKAMAK_RUNTIME = "true";

  assert.equal(acceptsRuntimeCalls(), true);
});

void test("accepts calls in development", () => {
  process.env.NODE_ENV = "development";

  assert.equal(acceptsRuntimeCalls(), true);
});

void test("refuses calls elsewhere", () => {
  assert.equal(acceptsRuntimeCalls(), false);

  process.env.TOKAMAK_RUNTIME = "1";
  assert.equal(acceptsRuntimeCalls(), false);
});
