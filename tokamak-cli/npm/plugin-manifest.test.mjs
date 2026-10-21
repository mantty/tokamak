import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

import { discoverPlugins } from "./dist/plugin-manifest.mjs";
import { createWorkerGraph } from "./dist/worker-graph.mjs";
import { project } from "./plugin-service-fixture.mjs";

test("workspace plugin packages resolve without publishing or exporting their manifest", t => {
  const { root, dependency } = project(t);
  const [plugin] = discoverPlugins(root);
  assert.equal(plugin.package, "example-widgets");
  assert.equal(plugin.api.module, path.join(dependency, "src/index.ts"));
  assert.equal(plugin.api.export, "widgets");
  assert.equal(plugin.client, "example-widgets/client");
  assert.equal(plugin.manifest, path.join(dependency, "tokamak-plugin.json"));
});

test("plugin IDs obey the native packager's reserved option namespaces", t => {
  const { root, dependency } = project(t);
  for (const id of ["ios-location", "session-store", "name", "platform-camera"]) {
    fs.writeFileSync(path.join(dependency, "tokamak-plugin.json"), JSON.stringify({
      schemaVersion: 2, id, api: { module: "./src/index.ts", export: "widgets" },
    }));
    assert.throws(() => discoverPlugins(root), /plugin id must not start with/);
  }
});

test("duplicate plugin identifiers fail with a useful package diagnostic", t => {
  const { root, dependency } = project(t);
  const file = path.join(root, "package.json");
  const app = JSON.parse(fs.readFileSync(file, "utf8"));
  app.dependencies["other-widgets"] = "1.0.0";
  fs.writeFileSync(file, JSON.stringify(app));
  fs.symlinkSync(dependency, path.join(root, "node_modules/other-widgets"), "dir");
  assert.throws(() => discoverPlugins(root), /Duplicate Tokamak plugin id widgets/);
});

test("module references cannot leave their package", t => {
  const { root, dependency } = project(t);
  fs.writeFileSync(path.join(dependency, "tokamak-plugin.json"), JSON.stringify({
    schemaVersion: 2, id: "widgets", api: { module: "../../main.ts", export: "widgets" },
  }));
  assert.throws(() => discoverPlugins(root), /api.module must be a file inside the plugin package/);
});

test("unknown versions and incomplete factory declarations fail at discovery", t => {
  const { root, dependency } = project(t);
  const file = path.join(dependency, "tokamak-plugin.json");
  fs.writeFileSync(file, JSON.stringify({ schemaVersion: 3, id: "widgets" }));
  assert.throws(() => discoverPlugins(root), /unsupported plugin schema version 3/);
  fs.writeFileSync(file, JSON.stringify({ schemaVersion: 2, id: "widgets" }));
  assert.throws(() => discoverPlugins(root), /api must name a module and export/);
});

test("generated plugin bindings reject conflicting user bindings", t => {
  const { root, graph } = project(t);
  assert.throws(() => graph.configure({
    name: "app", main: "main.ts", configPath: path.join(root, "wrangler.json"),
    services: [{ binding: "TOKAMAK_WIDGETS", service: "elsewhere" }],
  }), /TOKAMAK_WIDGETS is reserved/);
});

test("published JavaScript plugins use their declaration files as the API contract", async t => {
  const { root, dependency } = project(t);
  const context = JSON.stringify(fileURLToPath(import.meta.resolve("@tokamakdev/plugin")));
  fs.writeFileSync(path.join(dependency, "index.mjs"), "export const widgets = ctx => ({ echo: value => value });");
  fs.writeFileSync(path.join(dependency, "index.d.mts"), `import type { TokamakContext } from ${context};
    export declare function widgets(ctx: TokamakContext): { echo(value: string): string };
  `);
  fs.writeFileSync(path.join(dependency, "tokamak-plugin.json"), JSON.stringify({
    schemaVersion: 2, id: "widgets", api: { module: "./index.mjs", export: "widgets" },
  }));
  const graph = createWorkerGraph(root, undefined, path.join(root, ".tokamak"));
  await graph.types.resolve(async id => ({ id }));
  assert.deepEqual(graph.types.plugins[0].methods, ["echo"]);
});

test("plugin factories can compose API interfaces and return classes with private state", async t => {
  for (const api of [
    `type A = { echo(value: string): string };
     type B = { connected(): boolean };
     export function widgets(ctx: TokamakContext): A & B {
       return { echo: value => value, connected: () => true };
     }`,
    `class Api {
       #connected = true;
       #value(value: string) { return value; }
       echo(value: string): string { return this.#value(value); }
       connected(): boolean { return this.#connected; }
     }
     export function widgets(ctx: TokamakContext) { return new Api(); }`,
  ]) {
    const { graph } = project(t, api);
    await graph.types.resolve(async id => ({ id }));
    assert.deepEqual(graph.types.plugins[0].methods, ["connected", "echo"]);
  }
});

test("each possible factory result is a synchronous API object", t => {
  for (const result of ["Api | Promise<Api>", "Api | (() => Api)"]) {
    assert.throws(() => project(t, `
      type Api = { echo(value: string): string };
      export function widgets(ctx: TokamakContext): ${result} { return { echo: value => value }; }
    `), /must return a synchronous object/);
  }
});

test("browser implementations must match the generated frontend API", async t => {
  for (const source of [
    "export const web = { async echo(value: number) { return value; } };",
    "export const web = { async missing() { return true; } };",
    "export const web = { echo(value: string) { return value; } };",
  ]) {
    const { dependency, graph } = project(t);
    fs.writeFileSync(path.join(dependency, "web.ts"), source);
    fs.writeFileSync(path.join(dependency, "tokamak-plugin.json"), JSON.stringify({
      schemaVersion: 2, id: "widgets", api: { module: "./src/index.ts", export: "widgets" },
      web: { module: "./web.ts", export: "web" },
    }));
    await assert.rejects(graph.types.resolve(async id => ({ id })), /web method/);
  }
});

test("manifest operations describe each operation's platforms and foreground needs", async t => {
  const { root, dependency } = project(t);
  const manifest = path.join(dependency, "tokamak-plugin.json");
  const write = operations => fs.writeFileSync(manifest, JSON.stringify({
    schemaVersion: 2, id: "widgets", api: { module: "./src/index.ts", export: "widgets" },
    platforms: { ios: {}, android: {} }, operations,
  }));
  write({ connected: { platforms: { ios: "17.0", android: "26" }, foreground: ["ios"] } });
  const [plugin] = discoverPlugins(root);
  assert.deepEqual(plugin.platforms, ["ios", "android"]);
  assert.deepEqual(plugin.operations, {
    connected: { platforms: [{ name: "ios", minimum: "17.0" }, { name: "android", minimum: "26" }], foreground: ["ios"] },
  });
  const graph = createWorkerGraph(root, undefined, path.join(root, ".tokamak"));
  await graph.types.resolve(async id => ({ id }));
  assert.deepEqual(graph.availability(), { widgets: { connected: ["ios", "android"], echo: ["ios", "android"] } });
  const types = fs.readFileSync(path.join(root, "tokamak-env.d.ts"), "utf8");
  assert.match(types, /Native: iOS 17\.0\+, Android API 26\+\. Web: not supported\. May need the app in the foreground on iOS, to show UI\./);
  assert.match(types, /Native: iOS, Android\. Web: not supported\./);
  assert.match(types, /readonly capabilities: Readonly<Record<"connected" \| "echo", boolean>>/);

  for (const [operations, message] of [
    [{ connected: { platforms: { macos: "14.0" } } }, /names macos, which the plugin does not build for/],
    [{ connected: { foreground: ["windows"] } }, /names windows, which the plugin does not build for/],
    [{ connected: { visible: true } }, /unknown keys: visible/],
    [{ connected: { platforms: { ios: 17 } } }, /must map platforms to a minimum OS version/],
  ]) {
    write(operations);
    assert.throws(() => discoverPlugins(root), message);
  }
  write({ missing: {} });
  assert.throws(() => createWorkerGraph(root, undefined, path.join(root, ".tokamak")), /operations\.missing names no method of widgets/);
});
