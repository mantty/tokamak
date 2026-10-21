import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import ts from "typescript";

import { createBuilder, createServer } from "vite";
import { project } from "./plugin-service-fixture.mjs";

test("installed plugin factories expose a named service without application forwarding methods", async t => {
  const { config } = project(t);
  const server = await createServer(config);
  t.after(() => server.close());
  await server.listen();
  const response = await fetch(server.resolvedUrls.local[0]);
  assert.equal(response.status, 200, await response.clone().text());
  assert.deepEqual(await response.json(), { value: "hello", connected: true });
});

test("plugin bindings and clients retain overloads and require checking native availability", async t => {
  const { root, graph } = project(t, `
    function echo(value: string): string;
    function echo(value: number): number;
    function echo(value: string | number): string | number { return value; }
    export function widgets(ctx: TokamakContext) { return { echo }; }
  `);
  await graph.types.resolve(async id => ({ id }));
  const file = path.join(root, "consumer.mts");
  fs.writeFileSync(file, `import client from "example-widgets/client";
    declare const env: Cloudflare.Env;
    const text: Promise<string> = client.echo("hello");
    const number: Promise<number> = client.echo(1);
    // @ts-expect-error requires a native binding
    env.TOKAMAK_WIDGETS.echo("hello");
    if (env.TOKAMAK_WIDGETS) {
      const text: Promise<string> = env.TOKAMAK_WIDGETS.echo("hello");
      // @ts-expect-error not a remote object
      env.TOKAMAK_WIDGETS.dup();
    }
    // @ts-expect-error no boolean overload
    client.echo(true);
    // @ts-expect-error clients have only exposed methods
    client.fetch("https://example.com");
  `);
  const program = ts.createProgram([file, path.join(root, "worker-configuration.d.ts"), path.join(root, "tokamak-env.d.ts")], {
    target: ts.ScriptTarget.ES2023, module: ts.ModuleKind.ESNext, moduleResolution: ts.ModuleResolutionKind.Bundler,
    strict: true, noEmit: true, types: [], lib: ["lib.es2023.d.ts"], allowImportingTsExtensions: true,
  });
  assert.deepEqual(ts.getPreEmitDiagnostics(program).map(diagnostic =>
    `${diagnostic.file?.fileName}:${diagnostic.start}: ${ts.flattenDiagnosticMessageText(diagnostic.messageText, "\n")}`), []);
});

test("the generated native bundle exports plugin services and the main config selects them", async t => {
  const { root, config } = project(t);
  const builder = await createBuilder(config);
  await builder.buildApp();
  const directory = path.join(root, ".wrangler/deploy");
  const deployment = JSON.parse(fs.readFileSync(path.join(directory, "config.json"), "utf8"));
  const main = JSON.parse(fs.readFileSync(path.resolve(directory, deployment.configPath), "utf8"));
  const nativePath = path.resolve(directory, deployment.auxiliaryWorkers[0].configPath);
  const native = JSON.parse(fs.readFileSync(nativePath, "utf8"));
  assert.deepEqual(main.services.find(service => service.binding === "TOKAMAK_WIDGETS"), {
    binding: "TOKAMAK_WIDGETS", service: native.name, entrypoint: "TOKAMAK_WIDGETS",
  });
  const source = fs.readFileSync(path.resolve(path.dirname(nativePath), native.main), "utf8");
  assert.match(source, /TOKAMAK_WIDGETS/);
});

test("a plugin callback signature is rejected before building its service", async t => {
  const { config } = project(t, `
    export function widgets(ctx: TokamakContext) {
      return { onChange(callback: () => void): void { callback(); } };
    }
  `);
  await assert.rejects(async () => (await createBuilder(config)).buildApp(), /onChange.callback/);
});

test("plugin methods that pass native references stay in the Tokamak worker", async t => {
  const sdk = JSON.stringify(fileURLToPath(import.meta.resolve("@tokamakdev/plugin")));
  const { root, graph } = project(t, `
    import type { NativeReference } from ${sdk};
    export function widgets(ctx: TokamakContext) {
      return {
        echo(value: string): string { return value; },
        capture(): Promise<NativeReference> { throw new Error("native"); },
        encode(frames: { frame: NativeReference }[]): Promise<number> { throw new Error("native"); },
      };
    }
  `);
  await graph.types.resolve(async id => ({ id }));
  assert.deepEqual(graph.types.plugins[0].methods, ["echo"]);
  assert.doesNotMatch(fs.readFileSync(path.join(root, "tokamak-env.d.ts"), "utf8"), /capture|encode/);
});

test("a plugin factory requires the receiving native invocation context", async t => {
  await assert.rejects(async () => {
    const { config } = project(t, `
      export function widgets(ctx: { waitUntil(promise: Promise<unknown>): void }) {
        return { echo(value: string): string { return value; } };
      }
    `);
    await (await createBuilder(config)).buildApp();
  }, /widgets.*TokamakContext/);
});

test("plugin API edits replace the named service method list during development", async t => {
  const { dependency, root, config } = project(t);
  const server = await createServer(config);
  t.after(() => server.close());
  await server.listen();
  const url = server.resolvedUrls.local[0];
  assert.equal((await fetch(url)).status, 200);
  const source = path.join(dependency, "src/index.ts");
  fs.writeFileSync(source, fs.readFileSync(source, "utf8").replace('echo(value: string): string { return value; }',
    'greet(value: string): string { return "hi " + value; }'));
  fs.writeFileSync(path.join(root, "main.ts"), fs.readFileSync(path.join(root, "main.ts"), "utf8")
    .replace('.echo("hello")', '.greet("hello")'));
  let result;
  for (const end = Date.now() + 5_000; Date.now() < end;) {
    const response = await fetch(url);
    if (response.ok) result = await response.json();
    if (result?.value === "hi hello") break;
    await new Promise(resolve => setTimeout(resolve, 25));
  }
  assert.deepEqual(result, { value: "hi hello", connected: true });
});
