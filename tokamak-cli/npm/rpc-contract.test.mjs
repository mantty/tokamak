import assert from "node:assert/strict";
import path from "node:path";
import { test } from "node:test";
import ts from "typescript";

import { checkRpcMethod } from "./dist/rpc-contract.mjs";

function contract(source, extension = "ts", ambient, types = []) {
  const file = path.resolve(`rpc-fixture.${extension}`);
  const globals = path.resolve("rpc-globals.d.ts");
  const options = {
    target: ts.ScriptTarget.ES2023,
    module: ts.ModuleKind.ESNext,
    strict: true,
    allowJs: true,
    checkJs: true,
    noEmit: true,
    types,
    lib: ambient ? ["lib.es2023.d.ts"] : ["lib.es2023.d.ts", "lib.dom.d.ts"],
  };
  const host = ts.createCompilerHost(options);
  const read = host.readFile;
  host.readFile = (name) => name === file ? source : name === globals ? ambient : read(name);
  const program = ts.createProgram(ambient ? [file, globals] : [file], options, host);
  const checker = program.getTypeChecker();
  const module = program.getSourceFile(file);
  const diagnostics = program.getSemanticDiagnostics(module).filter(({ code }) => code !== 7006);
  assert.deepEqual(diagnostics.map(({ messageText }) => ts.flattenDiagnosticMessageText(messageText, "\n")), []);
  const symbol = checker.getSymbolAtLocation(module);
  const methods = checker.getExportsOfModule(symbol);
  return (name) => {
    const method = methods.find((method) => method.name === name);
    assert.ok(method, `Missing fixture method ${name}`);
    const type = checker.getTypeOfSymbolAtLocation(method, method.valueDeclaration);
    checkRpcMethod(checker, type, name);
  };
}

const valid = contract(`
type Tree = { value: string | number; children?: readonly Tree[] };
type Binary = ArrayBuffer | Uint8Array | DataView | BigInt64Array;
declare const brand: unique symbol;
type ID = string & { readonly [brand]: "id" };
type Count = number & { readonly [brand]: "count" };
class Failure extends Error { code = 42; }
class GenericFailure<T> extends Error { constructor(public details: T) { super(); } }
export function data(value: [Tree, Map<string, Set<Date>>, RegExp, Binary, bigint, null, boolean]) { return value; }
export function dictionary(values: Record<string, number | undefined>) { return values; }
export function mapped(values: Record<"a" | "b", string>) { return values; }
export function remapped(values: { [Key in "a" | "b" as Uppercase<Key>]: number }) { return values; }
export function stringKeys(values: { "__@field": string }) { return values; }
export function branded(value: [ID, Count]) { return value; }
export async function streams(source: ReadableStream<Uint8Array>): Promise<{ body: ReadableStream<Uint8Array> }> { return { body: source }; }
export async function nothing(): Promise<void> {}
export function rest(...values: readonly [string, ...number[]]) { return values; }
export function fail(): never { throw new Error("failed"); }
export function errors(error: Error | DOMException) { return error; }
export function failures(error: Failure | TypeError | AggregateError) { return error; }
export function genericFailure(error: GenericFailure<string>) { return error; }
export function readonly(value: ReadonlyMap<string, ReadonlySet<string>>) { return value; }
export function optionalReturn(value: boolean): void | string { if (value) return "yes"; }
export function accessor(value: { get item(): string }) { return value; }
export function concrete<T>(value: T): T { return value; }
export const specialized = concrete<string>;
export function overload(value: string): string;
export function overload(value: number): number;
export function overload(value: string | number) { return value; }
`);

for (const name of ["data", "dictionary", "mapped", "remapped", "stringKeys", "branded", "streams", "nothing", "rest", "fail", "errors", "failures", "genericFailure", "readonly", "optionalReturn", "accessor", "specialized", "overload"]) {
  test(`accepts the ${name} RPC signature`, () => valid(name));
}

const invalid = contract(`
class RpcTarget {}
class Value { value = 1; }
class BadError extends Error { callback = () => {}; }
class Box<T> { constructor(public value: T) {} }
interface Stub { fetch(request: Request): Promise<Response> }
interface Recursive { next?: Recursive; callback?: () => void }
export function callback(options: { listener: () => void }) {}
export function target(): RpcTarget { return new RpcTarget(); }
export function stub(value: Stub) {}
export function instance(value: Value) {}
export function badError(value: BadError) {}
export function classInstance(value: Box<string>) {}
export function shared(value: SharedArrayBuffer) {}
export function sharedView(value: Uint8Array<SharedArrayBuffer>) {}
export function untypedView(value: Uint8Array<any>) {}
export function writable(value: WritableStream<Uint8Array>) {}
export function transform(value: TransformStream<Uint8Array, Uint8Array>) {}
export function objects(value: ReadableStream<{ value: string }>) {}
export function nested(value: { result: Promise<string> }) {}
export function request(value: Request) {}
export function response(): Response { return new Response(); }
export function symbol(value: symbol) {}
export function weak(value: WeakMap<object, string>) {}
export function anything(value: any) {}
export function unknown(value: unknown) {}
export function object(value: object) {}
export function empty(value: {}) {}
export function generic<T>(value: T) { return value; }
export function genericReturn<T>(): T { throw new Error(); }
export function recursive(value: Recursive) {}
export function union(value: string | (() => void)) {}
export function nestedAny(value: Map<string, any[]>) {}
export function wrongOverload(value: string): string;
export function wrongOverload(value: () => void): void;
export function wrongOverload(value: string | (() => void)) { if (typeof value === "string") return value; }
export function symbolField(value: { [Symbol.iterator]: string }) {}
export function symbolIndex(value: { [key: symbol]: string }) {}
export function growing<T>(value: { next: T }): T { throw new Error(); }
`);

for (const [name, location] of [
  ["callback", "callback.options.listener"], ["target", "target.result"],
  ["stub", "stub.value.fetch"], ["instance", "instance.value"],
  ["badError", "badError.value.callback"],
  ["classInstance", "classInstance.value"],
  ["shared", "shared.value"], ["sharedView", "sharedView.value"],
  ["untypedView", "untypedView.value"],
  ["writable", "writable.value"], ["transform", "transform.value"],
  ["objects", "objects.value"], ["nested", "nested.value.result"],
  ["request", "request.value"], ["response", "response.result"],
  ["symbol", "symbol.value"], ["weak", "weak.value"],
  ["anything", "anything.value"], ["unknown", "unknown.value"],
  ["object", "object.value"], ["empty", "empty.value"],
  ["generic", "generic"], ["genericReturn", "genericReturn"],
  ["recursive", "recursive.value.callback"], ["union", "union.value"],
  ["nestedAny", "nestedAny.value"], ["wrongOverload", "wrongOverload.value"],
  ["symbolField", "symbolField.value"], ["symbolIndex", "symbolIndex.value"],
  ["growing", "growing"],
]) {
  test(`rejects ${name} with its contract location`, () => {
    assert.throws(() => invalid(name), (error) => {
      assert.match(error.message, /Unsupported RPC/);
      assert.ok(error.message.includes(location), error.message);
      return true;
    });
  });
}

test("accepts checked JavaScript signatures from JSDoc", () => {
  const check = contract(`/** @param {{ longitude: number, latitude: number }} point */
    export async function locate(point) { return point; }`, "js");
  assert.doesNotThrow(() => check("locate"));
});

test("rejects JavaScript parameters without a concrete type", () => {
  const check = contract("export function call(value) { return value; }", "js");
  assert.throws(() => check("call"), /call.value/);
});

test("does not treat an application class named Date as the built-in Date", () => {
  const check = contract("class Date {} export function call(value: Date) {}");
  assert.throws(() => check("call"), /call.value/);
});

test("accepts worker globals without the browser DOM library", () => {
  const check = contract(`export function stream(value: ReadableStream<Uint8Array>) { return value; }
    export function error(value: DOMException) { return value; }`, "ts", `
    declare class ReadableStream<R = any> { cancel(reason?: any): Promise<void>; }
    declare class DOMException extends Error { constructor(message?: string, name?: string); stack: any; readonly code: number; }
  `);
  check("stream");
  check("error");
});

test("accepts ReadableStream imported from node:stream/web", () => {
  const check = contract(`import { ReadableStream } from "node:stream/web";
    export function stream(value: ReadableStream<Uint8Array>) { return value; }`, "ts", undefined, ["node"]);
  check("stream");
});

test("reports indefinitely expanding generic shapes without overflowing the stack", () => {
  const check = contract(`type Grow<T> = { value: T; next?: Grow<T[]> };
    export function grow(value: Grow<string>) {}`);
  assert.throws(() => check("grow"), /Unsupported RPC type at grow.value.*type expansion is too deep/);
});
