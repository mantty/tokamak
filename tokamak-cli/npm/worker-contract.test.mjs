import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import ts from "typescript";

import { readWorkerContract, readServiceContracts } from "./dist/worker-contract.mjs";
import { workerTypes } from "./worker-fixtures.mjs";

function project(t, source, files = {}, read = readWorkerContract) {
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
  return () => read(path.join(root, "src/tokamak.ts"), root);
}

function checkClient(contract, source) {
  const entry = contract.files.find((file) => file.endsWith("/src/tokamak.ts"));
  const root = path.dirname(path.dirname(entry));
  const declaration = path.join(root, "client.d.mts");
  const consumer = path.join(root, "consumer.mts");
  fs.writeFileSync(declaration, contract.declarations);
  fs.writeFileSync(consumer, `import type { TokamakClient, TokamakService } from "./client.mjs";
    declare const client: TokamakClient;
    declare const service: TokamakService;
    ${source}`);
  const config = ts.findConfigFile(root, ts.sys.fileExists);
  const options = config ? ts.parseJsonConfigFileContent(
    ts.readConfigFile(config, ts.sys.readFile).config, ts.sys, path.dirname(config),
  ).options : {};
  const program = ts.createProgram([consumer, path.join(root, "worker-configuration.d.ts")], {
    ...options, target: ts.ScriptTarget.ES2023, module: ts.ModuleKind.ESNext,
    moduleResolution: ts.ModuleResolutionKind.Bundler,
    strict: true, allowJs: true, checkJs: true, noEmit: true, types: [], lib: ["lib.es2023.d.ts"],
  });
  const diagnostics = ts.getPreEmitDiagnostics(program);
  assert.deepEqual(diagnostics.map((diagnostic) => ts.flattenDiagnosticMessageText(diagnostic.messageText, "\n")), []);
}

test("generated clients preserve overloads, recursive private types, brands, and asynchronous results", (t) => {
  const read = project(t, `
    declare const brand: unique symbol;
    type ID = string & { readonly [brand]: "id" };
    interface Tree { id: ID; children: Tree[] }
    class Base<T> extends WorkerEntrypoint { echo(value: T): T { return value; } }
    export default class Native extends Base<string> {
      parse(value: string): number;
      parse(value: number, radix?: number): string;
      parse(value: string | number, radix?: number) { return typeof value === "string" ? Number(value) : value.toString(radix); }
      tree(value: Tree): Tree { return value; }
      id(): ID { return "id" as ID; }
      async upload(body: ReadableStream<Uint8Array>): Promise<bigint> { return 1n; }
      collect(prefix: string, ...values: number[]): number[] { return values; }
      finish(): void {}
      fail(): never { throw new Error("failed"); }
      #helper() {}
      field = () => 1;
      get local() { return 1; }
      event(): void {}
    }
  `);
  checkClient(read(), `
    const number: Promise<number> = client.parse("12");
    const text: Promise<string> = client.parse(12, 10);
    const echo: Promise<string> = client.echo("value");
    const tree = await client.tree({ id: await client.id(), children: [] });
    await client.tree(tree.children[0]);
    const bytes: Promise<bigint> = client.upload(new ReadableStream<Uint8Array>());
    const collected: Promise<number[]> = client.collect("prefix", 1, 2);
    const done: Promise<void> = client.finish();
    const never: Promise<never> = client.fail();
    const response: Promise<Response> = service.fetch("https://app.example", { method: "POST" });
    const result: Promise<number> = service.parse("12");
    // @ts-expect-error the argument selects the overload's result
    const mismatch: Promise<number> = client.parse(12);
    // @ts-expect-error private brands remain distinct from ordinary strings
    client.tree({ id: "ordinary", children: [] });
    // @ts-expect-error calls are always asynchronous
    const sync: number = client.parse("12");
    // @ts-expect-error invalid overload arguments
    client.parse(true);
    // @ts-expect-error fields are local
    client.field();
    // @ts-expect-error getters are local
    client.local;
    // @ts-expect-error events are host-only
    client.event();
    // @ts-expect-error fetch belongs to service bindings
    client.fetch("https://app.example");
    // @ts-expect-error promise pipelining is outside the RPC contract
    client.tree(tree).id;
  `);
});

test("generated clients retain every overload without a fixed overload limit", (t) => {
  const overloads = Array.from({ length: 12 }, (_, index) => `value(input: ${index}): ${index};`).join("\n");
  const read = project(t, `export default class Native extends WorkerEntrypoint {
    ${overloads}
    value(input: number): number { return input; }
  }`);
  checkClient(read(), Array.from({ length: 12 }, (_, index) =>
    `const result${index}: Promise<${index}> = client.value(${index});`).join("\n"));
});

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
  checkClient(contract, `
    const result: Promise<{ longitude: number; latitude: number }> = client.position({ longitude: 1, latitude: 2 });
    // @ts-expect-error the re-exported operation retains its imported parameter type
    client.position({ longitude: 1 });
  `);
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
  const valid = read();
  assert.deepEqual(valid.methods, ["echo"]);
  checkClient(valid, `
    const result: Promise<string> = client.echo("value");
    // @ts-expect-error JSDoc parameter types reach the generated client
    client.echo(1);
  `);
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

test("ignores framework handlers but validates application RPC methods", (t) => {
  const read = project(t, "export default class Native extends WorkerEntrypoint { ping() { return 1; } }");
  assert.deepEqual(read().methods, ["ping"]);
  const invalid = project(t, "export default class Native extends WorkerEntrypoint { ping(callback: () => void) {} }");
  assert.throws(invalid, /Unsupported RPC type at ping.callback/);
});

test("keeps class-based platform event handlers outside the RPC contract", (t) => {
  const read = project(t, `export default class Main extends WorkerEntrypoint {
    queue(batch: MessageBatch<string>): void { batch.ackAll(); }
    scheduled(controller: ScheduledController): void { controller.noRetry(); }
    echo(value: string): string { return value; }
  }`, {}, readServiceContracts);
  assert.deepEqual(read().default.methods, ["echo"]);
});

test("accepts the Tokamak entrypoint and excludes its host event handler", (t) => {
  const sdk = JSON.stringify(fileURLToPath(import.meta.resolve("@tokamakdev/tok")));
  const read = project(t, `import { TokamakEntrypoint } from ${sdk};
    export default class Native extends TokamakEntrypoint {
      echo(value: string): string { return value; }
    }`);
  assert.deepEqual(read().methods, ["echo"]);
});

test("discovers default and named Worker services while preserving other module exports", (t) => {
  const read = project(t, `
    export default { fetch(request: Request) { return new Response(request.url); } };
    export class Recipes extends WorkerEntrypoint { title(id: number): string { return String(id); } }
    export class LocalHelper { callback(value: () => void) {} }
    export const label = "application";
  `, {}, readServiceContracts);
  const services = read();
  assert.deepEqual(Object.keys(services).sort(), ["Recipes", "default"]);
  assert.deepEqual(services.default.methods, []);
  assert.deepEqual(services.Recipes.methods, ["title"]);
});

test("validates main and named service signatures using the same RPC contract", (t) => {
  for (const name of ["default", "Other"]) {
    const source = `${name === "default" ? "export default" : "export"} class ${name === "default" ? "Main" : name} extends WorkerEntrypoint {
      call(value: () => void) {}
    }`;
    assert.throws(project(t, source, {}, readServiceContracts), /Unsupported RPC type at call.value/);
  }
  assert.throws(project(t, `export default class Main extends WorkerEntrypoint {
    __tokamakRpc(): void {}
  }`), /__tokamakRpc.*reserved/);
});
