#!/usr/bin/env node
// Regenerates tokamak/src/streams/node.mjs from the readable-stream npm package
// (Node core streams, extracted upstream for portability). Run with:
//
//   node scripts/vendor-readable-stream.mjs
//
// The script installs pinned versions into a temporary directory, bundles the
// node:stream surface with esbuild (externalizing the runtime builtins the
// bundle needs), verifies the result behaves like Node streams under the local
// Node.js, and only then rewrites tokamak/src/streams/node.mjs.

import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import path from "node:path";
import { pathToFileURL } from "node:url";

const READABLE_STREAM_VERSION = "4.7.0";
const ESBUILD_VERSION = "0.28.2";
const NODE_ADAPTERS_VERSION = "v22.18.0";
const OUTPUT = path.join(import.meta.dirname, "..", "tokamak", "src", "streams", "node.mjs");

// The only module specifiers the generated bundle may import. Each one is a
// builtin the Tokamak runtime resolves (see tokamak/src/runtime_modules.rs).
const ALLOWED_IMPORTS = new Set(["node:buffer", "node:events", "node:process", "node:string_decoder"]);

// readable-stream requires bare "buffer", "events", "string_decoder/" and
// "process/". Each is redirected to a small ESM shim so the bundle ends up
// importing the runtime's builtins instead of npm polyfills.
const SHIMS = {
  "buffer.mjs": 'export * from "node:buffer";\nexport { default } from "node:buffer";\n',
  "events.mjs": 'export * from "node:events";\nexport { default } from "node:events";\n',
  "string-decoder.mjs": 'export * from "node:string_decoder";\nexport { default } from "node:string_decoder";\n',
  // CommonJS interop resolves properties on the module namespace, so nextTick
  // must be a named export. The wrapper reads process.nextTick lazily: the
  // runtime installs it after module evaluation.
  "process.mjs": [
    'import process from "node:process";',
    "export default process;",
    "export const nextTick = (callback, ...args) => process.nextTick(callback, ...args);",
    "",
  ].join("\n"),
  // The runtime provides web AbortController/AbortSignal globals, so the
  // abort-controller polyfill package is never needed.
  "abort-controller.mjs": [
    "export const AbortController = globalThis.AbortController;",
    "export const AbortSignal = globalThis.AbortSignal;",
    "",
  ].join("\n"),
};

// readable-stream omits the web adapters. Restore their shared Node/Workerd
// implementation using readable-stream's existing internals, not another
// stream implementation. Only the six public adapters are bundled; Node's
// native StreamBase adapters do not apply to Tokamak's transport.
async function installWebAdapters(buildDir) {
  const response = await fetch(`https://raw.githubusercontent.com/nodejs/node/${NODE_ADAPTERS_VERSION}/lib/internal/webstreams/adapters.js`);
  assert.ok(response.ok, `fetch adapters: ${response.status}`);
  const source = await response.text();
  assert.equal(createHash("sha256").update(source).digest("hex"), "4fe34b10c4cdd0b3c8e22f5b763b4930d54500b908be617d961c0b9f8715d1c2");
  const start = source.indexOf("function newWritableStreamFromStreamWritable(");
  const end = source.lastIndexOf("/**", source.indexOf("function newWritableStreamFromStreamBase("));
  assert.ok(start > 0 && end > start);
  let adapters = source.slice(start, end);
  // The upstream vector callback assumes even a rejection is an array and
  // throws before calling Node's write callbacks. Separate success/error
  // settlement; the compiled-runtime tests cover both Writable and Duplex.
  for (const [before, after] of [
    ["        error = error.filter((e) => e);\n", ""],
    ["callback(error.length === 0 ? undefined : error);", "callback(error);"],
    ["(data) => writer.write(data.chunk)),\n            done,", "(data) => writer.write(data.chunk)),\n            () => done(),"],
  ]) {
    assert.equal(adapters.split(before).length, 3, "both vector adapters contain patch target");
    adapters = adapters.replaceAll(before, after);
  }
  // An errored side is closed; its peer still needs cancellation in _destroy.
  // Marking both closed here skips that cleanup and leaks the live source/sink.
  const closedPair = "      writableClosed = true;\n      readableClosed = true;\n      destroy(duplex, error);";
  assert.equal(adapters.split(closedPair).length, 3, "both duplex rejection handlers contain patch target");
  adapters = adapters.replace(closedPair, "      writableClosed = true;\n      destroy(duplex, error);");
  adapters = adapters.replace(closedPair, "      readableClosed = true;\n      destroy(duplex, error);");
  const prelude = `
const { Readable, Writable, Duplex, destroy } = require('../../stream');
const { Buffer } = require('buffer');
const process = require('process/');
const { isDestroyed, isReadable, isWritable, isWritableEnded } = require('../streams/utils');
const finished = require('../streams/end-of-stream');
const { AbortError, codes: { ERR_INVALID_ARG_TYPE, ERR_INVALID_ARG_VALUE, ERR_STREAM_PREMATURE_CLOSE } } = require('../../ours/errors');
const { validateBoolean, validateObject } = require('../validators');
const { kEmptyObject } = require('../../ours/util');
const normalizeEncoding = encoding => encoding.toLowerCase().replace(/^utf-8$/, 'utf8');
const { PromisePrototypeThen, PromiseResolve } = require('../../ours/primordials');
const PromiseWithResolvers = Promise.withResolvers.bind(Promise);
const SafePromiseAll = (values, mapper) => Promise.all(mapper ? values.map(mapper) : values);
const SafePromisePrototypeFinally = Function.prototype.call.bind(Promise.prototype.finally);
const TypedArrayPrototypeGetBuffer = value => value.buffer;
const TypedArrayPrototypeGetByteOffset = value => value.byteOffset;
const TypedArrayPrototypeGetByteLength = value => value.byteLength;
const isReadableStream = value => value instanceof ReadableStream;
const isWritableStream = value => value instanceof WritableStream;
const encoder = new TextEncoder();
function handleKnownInternalErrors(cause) {
  if (cause?.code === 'ERR_STREAM_PREMATURE_CLOSE') return new AbortError(undefined, { cause });
  if (['Z_ERRNO', 'Z_STREAM_ERROR', 'Z_DATA_ERROR', 'Z_MEM_ERROR', 'Z_BUF_ERROR', 'Z_VERSION_ERROR', 'Z_NEED_DICT'].includes(cause?.code)) {
    const error = new TypeError(undefined, { cause });
    error.code = cause.code;
    return error;
  }
  return cause;
}
`;
  const exports = `module.exports = {
  newWritableStreamFromStreamWritable, newStreamWritableFromWritableStream,
  newReadableStreamFromStreamReadable, newStreamReadableFromReadableStream,
  newReadableWritablePairFromDuplex, newStreamDuplexFromReadableWritablePair,
};\n`;
  const packageRoot = path.join(buildDir, 'node_modules/readable-stream/lib');
  mkdirSync(path.join(packageRoot, 'internal/webstreams'), { recursive: true });
  writeFileSync(path.join(packageRoot, 'internal/webstreams/adapters.js'), prelude + adapters + exports);
  for (const name of ['readable', 'writable', 'duplex']) {
    const filename = path.join(packageRoot, 'internal/streams', `${name}.js`);
    const original = readFileSync(filename, 'utf8');
    const stub = 'webStreamsAdapters = {}';
    assert.equal(original.split(stub).length, 2, `one adapter stub in ${name}`);
    writeFileSync(filename, original.replace(stub, "webStreamsAdapters = require('../webstreams/adapters')"));
  }
}

// Bundle entry: readable-stream's node:stream surface plus the pieces Node
// exports that readable-stream lacks, plus legacy named exports kept for the
// tokamak _stream_* shims.
const ENTRY = `import Stream from "readable-stream/lib/stream.js";
import promises from "readable-stream/lib/stream/promises.js";

const {
  Readable, Writable, Duplex, Transform, PassThrough,
  addAbortSignal, compose, destroy, finished, pipeline,
  isDestroyed, isDisturbed, isErrored, isReadable, isWritable,
  getDefaultHighWaterMark, setDefaultHighWaterMark,
  _isUint8Array, _uint8ArrayToBuffer,
} = Stream;

// Node exports these on node:stream but readable-stream does not provide them.
function _isArrayBufferView(value) {
  return ArrayBuffer.isView(value);
}

// Port of Node's lib/internal/streams/duplexpair.js.
const kCallback = Symbol("Callback");
const kOtherSide = Symbol("Other");

class DuplexSide extends Duplex {
  constructor(options) {
    super(options);
    this[kCallback] = null;
    this[kOtherSide] = null;
  }

  _read() {
    const callback = this[kCallback];
    if (callback) {
      this[kCallback] = null;
      callback();
    }
  }

  _write(chunk, encoding, callback) {
    if (chunk.length === 0) {
      process.nextTick(callback);
    } else {
      this[kOtherSide].push(chunk);
      this[kOtherSide][kCallback] = callback;
    }
  }

  _final(callback) {
    this[kOtherSide].on("end", callback);
    this[kOtherSide].push(null);
  }
}

function duplexPair(options) {
  const side0 = new DuplexSide(options);
  const side1 = new DuplexSide(options);
  side0[kOtherSide] = side1;
  side1[kOtherSide] = side0;
  return [side0, side1];
}

Stream._isArrayBufferView = _isArrayBufferView;
Stream.duplexPair = duplexPair;

// Legacy tokamak names kept for the node/internal-stream-*.mjs shims and any
// existing importers. Node's node:stream does not export these.
class Wrapper extends Duplex {}
const { ReadableState, _fromList, from, fromWeb, toWeb } = Readable;
const { WritableState } = Writable;
const consumers = {};

export {
  Stream, Readable, Writable, Duplex, Transform, PassThrough, Wrapper,
  ReadableState, WritableState,
  addAbortSignal, compose, destroy, duplexPair, finished, pipeline, promises,
  isDestroyed, isDisturbed, isErrored, isReadable, isWritable,
  getDefaultHighWaterMark, setDefaultHighWaterMark,
  _fromList, _isArrayBufferView, _isUint8Array, _uint8ArrayToBuffer,
  consumers, from, fromWeb, toWeb,
};
export default Stream;
`;

// Own enumerable keys of Node v22's node:stream module object. The
// differential contract test compares Object.keys() of the default export
// against workerd, so the generated default must match exactly.
const NODE_STREAM_KEYS = [
  "Duplex", "PassThrough", "Readable", "Stream", "Transform", "Writable",
  "_isArrayBufferView", "_isUint8Array", "_uint8ArrayToBuffer",
  "addAbortSignal", "compose", "destroy", "duplexPair", "finished",
  "getDefaultHighWaterMark", "isDestroyed", "isDisturbed", "isErrored",
  "isReadable", "isWritable", "pipeline", "promises", "setDefaultHighWaterMark",
];

const NAMED_EXPORTS = [
  "Duplex", "PassThrough", "Readable", "ReadableState", "Stream", "Transform",
  "Wrapper", "Writable", "WritableState", "_fromList", "_isArrayBufferView",
  "_isUint8Array", "_uint8ArrayToBuffer", "addAbortSignal", "compose",
  "consumers", "default", "destroy", "duplexPair", "finished", "from",
  "fromWeb", "getDefaultHighWaterMark", "isDestroyed", "isDisturbed",
  "isErrored", "isReadable", "isWritable", "pipeline", "promises",
  "setDefaultHighWaterMark", "toWeb",
];

function verifyImports(bundle) {
  assert.ok(!bundle.includes("Dynamic require of"), "bundle contains an unresolved CommonJS require");
  const specifiers = new Set();
  for (const match of bundle.matchAll(/^import\b[^"']*["']([^"']+)["'];?$/gm)) specifiers.add(match[1]);
  for (const specifier of specifiers) {
    assert.ok(ALLOWED_IMPORTS.has(specifier), `bundle imports unexpected module "${specifier}"`);
  }
}

async function verifyBehavior(bundlePath) {
  const stream = await import(pathToFileURL(bundlePath));
  const { Readable, Writable, Transform, duplexPair, finished, pipeline, promises } = stream;

  assert.deepEqual(Object.keys(stream.default).sort(), NODE_STREAM_KEYS, "default export key set");
  for (const name of NAMED_EXPORTS) assert.ok(name in stream, `missing named export ${name}`);
  assert.equal(stream.default.promises, promises, "stream.promises identity");

  // Readable.from delivers the iterable.
  const collected = [];
  for await (const chunk of Readable.from(["a", "b"])) collected.push(chunk);
  assert.deepEqual(collected, ["a", "b"], "Readable.from round trip");

  // Callback pipeline flows data through a Transform into a Writable.
  const out = [];
  await new Promise((resolve, reject) => {
    pipeline(
      Readable.from(["a", "b"]),
      new Transform({
        transform(chunk, encoding, callback) { callback(null, chunk.toString().toUpperCase()); },
      }),
      new Writable({
        write(chunk, encoding, callback) { out.push(chunk.toString()); callback(); },
      }),
      error => error ? reject(error) : resolve(),
    );
  });
  assert.deepEqual(out, ["A", "B"], "pipeline delivery");

  // finished() settles on an already-ended stream.
  const drained = Readable.from(["x"]);
  for await (const chunk of drained) void chunk;
  await new Promise((resolve, reject) => finished(drained, error => error ? reject(error) : resolve()));

  // promises API works and honors transform delivery.
  const promiseOut = [];
  await promises.pipeline(
    Readable.from(["p"]),
    new Writable({ write(chunk, encoding, callback) { promiseOut.push(chunk.toString()); callback(); } }),
  );
  assert.deepEqual(promiseOut, ["p"], "promises.pipeline delivery");

  // Subclass with _transform (node/crypto.mjs pattern).
  const upper = new (class extends Transform {
    _transform(chunk, encoding, callback) { callback(null, chunk.toString().toUpperCase()); }
  })();
  const seen = [];
  upper.on("data", chunk => seen.push(chunk.toString()));
  upper.end("hi");
  await new Promise(resolve => upper.on("end", resolve));
  assert.deepEqual(seen, ["HI"], "Transform subclass with _transform");

  // Instance flag assignment (node/http.mjs pattern) must not throw.
  const flagged = new Readable({ read() {} });
  flagged.readable = true;

  // duplexPair carries writes across to the other side.
  const [side0, side1] = duplexPair();
  const across = [];
  side1.on("data", chunk => across.push(chunk.toString()));
  side0.write("ping");
  side0.end();
  await new Promise(resolve => side1.on("end", resolve));
  assert.deepEqual(across, ["ping"], "duplexPair delivery");

  const webChunks = [];
  const writer = Writable.toWeb(new Writable({
    write(chunk, encoding, callback) { webChunks.push(...chunk); callback(); },
  })).getWriter();
  await writer.write(new Uint8Array([1, 2]));
  await writer.close();
  assert.deepEqual(webChunks, [1, 2], "web adapter writes and closes");
}

const buildDir = mkdtempSync(path.join(tmpdir(), "tokamak-vendor-streams-"));
try {
  writeFileSync(path.join(buildDir, "package.json"), JSON.stringify({
    private: true,
    dependencies: { "readable-stream": READABLE_STREAM_VERSION, esbuild: ESBUILD_VERSION },
  }, null, 2));
  execFileSync("npm", ["install", "--cache", path.join(buildDir, "cache"), "--no-audit", "--no-fund", "--ignore-scripts", "--loglevel=error"], {
    cwd: buildDir,
    stdio: "inherit",
  });
  await installWebAdapters(buildDir);

  const shimDir = path.join(buildDir, "shims");
  mkdirSync(shimDir);
  for (const [name, source] of Object.entries(SHIMS)) writeFileSync(path.join(shimDir, name), source);
  writeFileSync(path.join(buildDir, "entry.mjs"), ENTRY);

  const shim = name => ({ path: path.join(shimDir, name) });
  const requireFromBuild = createRequire(path.join(buildDir, "package.json"));
  const { build } = requireFromBuild("esbuild");
  const bundlePath = path.join(buildDir, "bundle.mjs");
  await build({
    absWorkingDir: buildDir,
    entryPoints: [path.join(buildDir, "entry.mjs")],
    outfile: bundlePath,
    bundle: true,
    format: "esm",
    platform: "neutral",
    target: "es2022",
    mainFields: ["main"],
    legalComments: "eof",
    plugins: [{
      name: "tokamak-builtins",
      setup(pluginBuild) {
        pluginBuild.onResolve({ filter: /^node:/ }, args => ({ path: args.path, external: true }));
        pluginBuild.onResolve({ filter: /^buffer$/ }, () => shim("buffer.mjs"));
        pluginBuild.onResolve({ filter: /^events$/ }, () => shim("events.mjs"));
        pluginBuild.onResolve({ filter: /^string_decoder\/?$/ }, () => shim("string-decoder.mjs"));
        pluginBuild.onResolve({ filter: /^process\/?$/ }, () => shim("process.mjs"));
        pluginBuild.onResolve({ filter: /^abort-controller$/ }, () => shim("abort-controller.mjs"));
      },
    }],
  });

  const bundle = readFileSync(bundlePath, "utf8");
  verifyImports(bundle);

  const header = [
    `// Generated by scripts/vendor-readable-stream.mjs from readable-stream@${READABLE_STREAM_VERSION}`,
    `// (Node core streams, MIT licensed: https://github.com/nodejs/readable-stream) using`,
    `// esbuild@${ESBUILD_VERSION}. Do not edit; rerun the script to regenerate.`,
    `// Web adapters: Node.js ${NODE_ADAPTERS_VERSION}, MIT licensed; see READABLE-STREAM-LICENSE.`,
    "",
  ].join("\n");
  const finalPath = path.join(buildDir, "node.mjs");
  writeFileSync(finalPath, header + bundle);
  await verifyBehavior(finalPath);

  writeFileSync(OUTPUT, readFileSync(finalPath));
  writeFileSync(path.join(path.dirname(OUTPUT), "READABLE-STREAM-LICENSE"), readFileSync(path.join(buildDir, "node_modules/readable-stream/LICENSE")));
  console.log(`Wrote ${OUTPUT} (${readFileSync(finalPath).length} bytes)`);
} finally {
  rmSync(buildDir, { recursive: true, force: true });
}
