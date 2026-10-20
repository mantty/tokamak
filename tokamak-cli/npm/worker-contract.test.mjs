import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

import { readWorkerContract } from "./dist/worker-contract.mjs";
import { workerTypes } from "./worker-fixtures.mjs";

function project(t, source, files = {}) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "tokamak-contract-"));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  for (const [name, text] of Object.entries({
    "src/tokamak.ts": `import { WorkerEntrypoint } from "cloudflare:workers";\n${source}`,
    "worker-configuration.d.ts": workerTypes,
    ...files,
  })) {
    const file = path.join(root, name);
    fs.mkdirSync(path.dirname(file), { recursive: true });
    fs.writeFileSync(file, text);
  }
  return () => readWorkerContract(path.join(root, "src/tokamak.ts"), root);
}

test("exposes public prototype methods, including inherited concrete signatures", (t) => {
  const read = project(t, `
    class Base<T> extends WorkerEntrypoint { echo(value: T): T { return value; } }
    export default class Native extends Base<string> {
      #helper(callback: () => void) { callback(); }
      static configure(callback: () => void) { callback(); }
      field = (callback: () => void) => callback();
      get value() { return () => {}; }
      fetch(request: Request) { return new Response(request.url); }
      event(payload: unknown) {}
      async position(): Promise<{ latitude: number; longitude: number }> {
        return { latitude: 1, longitude: 2 };
      }
      "named-method"(value: Uint8Array) { return value; }
    }
  `);
  assert.deepEqual(read().methods, ["echo", "named-method", "position"]);
});

test("resolves project aliases, re-exports, and third-party declarations", (t) => {
  const read = project(t, 'export { Native as default } from "@app/native";', {
    "tsconfig.json": JSON.stringify({ extends: "./tsconfig.base.json" }),
    "tsconfig.base.json": JSON.stringify({ compilerOptions: {
      baseUrl: ".", paths: { "@app/*": ["src/*"] }, types: []
    } }),
    "src/native.ts": `import type { Position } from "location-types";
      import { WorkerEntrypoint as Entry } from "cloudflare:workers";
      export class Native extends Entry { position(value: Position) { return value; } }`,
    "node_modules/location-types/package.json": JSON.stringify({ name: "location-types", types: "index.d.ts" }),
    "node_modules/location-types/index.d.ts": "export interface Position { longitude: number; latitude: number }",
  });
  const contract = read();
  assert.deepEqual(contract.methods, ["position"]);
  assert.ok(contract.files.some((file) => file.endsWith("src/native.ts")));
  assert.ok(contract.files.some((file) => file.endsWith("location-types/index.d.ts")));
  assert.ok(contract.files.some((file) => file.endsWith("tsconfig.json")));
  assert.ok(contract.files.some((file) => file.endsWith("tsconfig.base.json")));
});

test("checks every overload and reports the offending argument", (t) => {
  const read = project(t, `export default class Native extends WorkerEntrypoint {
    run(value: string): string;
    run(value: () => void): void;
    run(value: string | (() => void)) { if (typeof value === "string") return value; }
  }`);
  assert.throws(read, /Unsupported RPC type at run.value/);
});

for (const access of ["private", "protected"]) {
  test(`rejects ${access} methods whose JavaScript prototype would expose them`, (t) => {
    const read = project(t, `export default class Native extends WorkerEntrypoint { ${access} secret() { return 1; } }`);
    assert.throws(read, /secret.*#private/);
  });
}

for (const name of ["connect", "then", "dup", "alarm", "webSocketMessage", "webSocketClose", "webSocketError"]) {
  test(`rejects the reserved ${name} operation`, (t) => {
    const read = project(t, `export default class Native extends WorkerEntrypoint { ${name}() { return 1; } }`);
    assert.throws(read, new RegExp(`${name}.*reserved`));
  });
}

for (const source of ["export const value = 1;", "export default {};", "export default function run() {}", "export default abstract class Native {}", "export default class Native<T> { echo(value: T) { return value; } }"]) {
  test(`requires a concrete default class: ${source}`, (t) => {
    assert.throws(project(t, source), /default.*concrete class/);
  });
}

test("accepts checked JavaScript and reports untyped exposed parameters", (t) => {
  const read = project(t, 'export { default } from "./native.js";', {
    "src/native.js": `import { WorkerEntrypoint } from "cloudflare:workers";
    export default class Native extends WorkerEntrypoint {
      /** @param {string} value */
      echo(value) { return value; }
    }`
  });
  assert.deepEqual(read().methods, ["echo"]);
  const invalid = project(t, 'export default class Native extends WorkerEntrypoint { echo(value) { return value; } }');
  assert.throws(invalid, /Unsupported RPC type at echo.value/);
});

test("requires the actual WorkerEntrypoint base, including through re-exports", (t) => {
  assert.throws(project(t, "export default class Native { ping() { return 1; } }"), /must extend.*Entrypoint/);
  const impostor = project(t, 'export { Native as default } from "./native";', {
    "src/native.ts": "class WorkerEntrypoint {} export class Native extends WorkerEntrypoint { ping() { return 1; } }",
  });
  assert.throws(impostor, /must extend.*Entrypoint/);
  const valid = project(t, 'export { Native as default } from "./native";', {
    "src/base.ts": 'export { WorkerEntrypoint as Base } from "cloudflare:workers";',
    "src/native.ts": 'import { Base } from "./base"; export class Native extends Base { ping() { return 1; } }',
  });
  assert.deepEqual(valid().methods, ["ping"]);
});

test("ignores inherited framework handlers but validates application overrides", (t) => {
  const read = project(t, "export default class Native extends WorkerEntrypoint { ping() { return 1; } }");
  assert.deepEqual(read().methods, ["ping"]);
  const invalid = project(t, "export default class Native extends WorkerEntrypoint { scheduled(callback: () => void) {} }");
  assert.throws(invalid, /Unsupported RPC type at scheduled.callback/);
});

test("accepts the Tokamak entrypoint and excludes its host event handler", (t) => {
  const sdk = JSON.stringify(fileURLToPath(import.meta.resolve("@tokamakdev/tok")));
  const read = project(t, `import { TokamakEntrypoint } from ${sdk};
    export default class Native extends TokamakEntrypoint {
      echo(value: string): string { return value; }
    }`);
  assert.deepEqual(read().methods, ["echo"]);
});
