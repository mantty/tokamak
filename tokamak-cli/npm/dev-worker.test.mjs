import assert from "node:assert/strict";
import { test } from "node:test";

import { DevWorker } from "./dist/dev-worker.mjs";

test("a disposed development worker rejects deliveries instead of starting a runtime", async () => {
  const worker = new DevWorker("app");
  await worker.dispose();
  await assert.rejects(worker.dispatch("example", new Uint8Array()), /development server has closed/);
});
