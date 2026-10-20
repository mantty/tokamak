import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { test } from "node:test";
import { pathToFileURL } from "node:url";
import { build as viteBuild } from "vite";
import WebSocket from "ws";

import { createClientPlugin } from "./dist/client.mjs";
import { project } from "./plugin-service-fixture.mjs";
import { readOutput, serve, withEnvironment } from "./vite-fixtures.mjs";

async function client(root, native) {
  const entry = path.join(root, "client.ts");
  fs.writeFileSync(entry, 'export { default } from "example-widgets/client";');
  const outDir = path.join(root, native ? "native-client" : "web-client");
  await viteBuild({
    root, configFile: false, logLevel: "silent",
    plugins: [createClientPlugin(undefined, native)],
    build: { outDir, lib: { entry, formats: ["es"], fileName: () => "client.mjs" } },
  });
  const file = path.join(outDir, "client.mjs");
  assert.doesNotMatch(fs.readFileSync(file, "utf8"), /cloudflare:workers|TokamakContext|nativeConnection|NATIVE_WIDGET_IMPLEMENTATION/);
  return (await import(pathToFileURL(file).href)).default;
}

test("a generated package client calls its plugin without an app-native entrypoint", async t => {
  const { root } = project(t, `
    export function widgets(ctx: TokamakContext) {
      return {
        echo(value: { count: bigint; bytes: Uint8Array }) { return value; },
        fail(): never { throw new Error("NATIVE_WIDGET_IMPLEMENTATION"); },
      };
    }
  `);
  const plugin = await client(root, true);
  const app = { root, output: path.join(root, ".tokamak/vite") };
  const originalSocket = globalThis.WebSocket;
  const originalLocation = globalThis.location;
  t.after(() => {
    globalThis.WebSocket = originalSocket;
    if (originalLocation === undefined) delete globalThis.location;
    else globalThis.location = originalLocation;
  });
  await withEnvironment({ TOKAMAK_SESSION_TOKEN: "plugin-client-test" }, () => serve(app, async () => {
    const { socketPort } = readOutput(app, "server.json");
    globalThis.location = new URL(`http://127.0.0.1:${socketPort}/page`);
    globalThis.WebSocket = class extends WebSocket {
      constructor(url) { super(url, { headers: {
        "x-tokamak-session": "plugin-client-test",
        "x-forwarded-host": "app.tokamak.local", "x-forwarded-proto": "https",
        origin: "https://app.tokamak.local",
      } }); }
    };
    const value = { count: 42n, bytes: new Uint8Array([1, 2]) };
    assert.deepEqual(await plugin.echo(value), value);
    await assert.rejects(plugin.fail(), { message: "NATIVE_WIDGET_IMPLEMENTATION" });
  }));
});

test("a native-only plugin client rejects unavailable calls on the web", async t => {
  const { root } = project(t);
  const plugin = await client(root, false);
  assert.deepEqual(Object.keys(plugin).sort(), ["connected", "echo"]);
  await assert.rejects(plugin.echo("hello"), { name: "NotSupportedError" });
});

test("web clients use declared browser methods and reject omitted operations", async t => {
  const { root, dependency } = project(t);
  fs.writeFileSync(path.join(dependency, "web.ts"), `class BrowserApi {
    #prefix = "BROWSER_WIDGET_IMPLEMENTATION ";
    async echo(value: string): Promise<string> { return this.#prefix + value; }
  }
  export const web = new BrowserApi();`);
  const manifest = path.join(dependency, "tokamak-plugin.json");
  fs.writeFileSync(manifest, JSON.stringify({
    ...JSON.parse(fs.readFileSync(manifest, "utf8")),
    web: { module: "./web.ts", export: "web" },
  }));
  const plugin = await client(root, false);
  assert.equal(await plugin.echo("hello"), "BROWSER_WIDGET_IMPLEMENTATION hello");
  await assert.rejects(plugin.connected(), { name: "NotSupportedError" });
  await client(root, true);
  assert.doesNotMatch(fs.readFileSync(path.join(root, "native-client/client.mjs"), "utf8"), /BROWSER_WIDGET_IMPLEMENTATION/);
});
