import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { afterEach, test } from "node:test";
import { fileURLToPath } from "node:url";

import { rollup } from "rollup";
import ts from "typescript";

import type { handlePushRequest } from "../worker/index.js";

const packages: Record<string, string | undefined> = {
  "@tokamakdev/plugin/worker": fileURLToPath(new URL("../../core/src/worker.ts", import.meta.url)),
};

/** Bundles the Worker helper as a production build does, with Rollup 4.63.0. */
async function productionBundle(): Promise<string> {
  const bundle = await rollup({
    input: fileURLToPath(new URL("../worker/index.ts", import.meta.url)),
    plugins: [
      {
        name: "production-typescript",
        resolveId: (source) => packages[source] ?? null,
        async load(id) {
          const source = await readFile(id, "utf8");
          const { outputText } = ts.transpileModule(source, {
            compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 },
          });
          return outputText.replaceAll("process.env.NODE_ENV", '"production"');
        },
      },
    ],
  });
  const { output } = await bundle.generate({ format: "es" });
  return output[0].code;
}

function push(): Request {
  return new Request("https://app.tokamak.local/tokamak/push", { method: "POST", body: "{}" });
}

afterEach(() => {
  Reflect.deleteProperty(globalThis, "__tokamak_runtime_call");
});

void test("keeps the runtime call check in a production Rollup 4.63.0 bundle", async () => {
  const code = await productionBundle();
  const bundled = (await import(`data:text/javascript,${encodeURIComponent(code)}`)) as {
    handlePushRequest: typeof handlePushRequest;
  };
  const accept = () => ({ id: "accepted", title: "Accepted" });

  assert.equal((await bundled.handlePushRequest(push(), accept)).status, 404);

  globalThis.__tokamak_runtime_call = true;
  assert.equal((await bundled.handlePushRequest(push(), accept)).status, 200);
});
