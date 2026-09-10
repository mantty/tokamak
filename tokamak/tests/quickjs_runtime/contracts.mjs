import EventEmitter from "node:events";
import streams from "node:stream";
import processModule from "node:process";
import { env, waitUntil } from "cloudflare:workers";

function result(callback) {
  try { return callback(); }
  catch (error) { return { error: error.name, dom: error instanceof DOMException, code: error.code ?? null }; }
}

export async function run(handlerEnv, ctx, constructors) {
  const output = {};
  output.identity = {
    events: process.getBuiltinModule("node:events") === EventEmitter,
    streams: process.getBuiltinModule("node:stream") === streams,
    process: processModule === process && process.getBuiltinModule("process") === process,
    bareEvents: process.getBuiltinModule("events") === EventEmitter,
    inherited: process.getBuiltinModule("toString") === undefined,
    env: env.FLAG === handlerEnv.FLAG,
    repeatedImport: (await import("node:events")).default === EventEmitter,
    globals: constructors.every((value, index) => value === [TextDecoder, TextEncoder, Response, ReadableStream, process][index]),
  };
  output.decode = {};
  for (const bytes of [[0xff], [0xe2, 0x28, 0xa1], [0xed, 0xa0, 0x80], [0xf4, 0x90, 0x80, 0x80], [0xc0, 0xaf], [0xe2, 0x82]]) {
    output.decode[bytes.join(",")] = new TextDecoder().decode(new Uint8Array(bytes));
  }
  const decoder = new TextDecoder();
  output.streaming = [
    decoder.decode(new Uint8Array([0xe2, 0x82]), { stream: true }),
    decoder.decode(new Uint8Array([0xac]), { stream: true }),
    decoder.decode(),
    decoder.decode(new Uint8Array([0xef, 0xbb, 0xbf, 65])),
  ];
  output.bom = [false, true].map(ignoreBOM => {
    const decoder = new TextDecoder("utf8", { ignoreBOM });
    return decoder.decode(new Uint8Array([0xef]), { stream: true }) + decoder.decode(new Uint8Array([0xbb, 0xbf, 65]));
  });
  output.encodings = {};
  for (const label of ["UTF8", " ascii ", "latin1", "windows-1252", "utf-16le", "utf-16be", "shift_jis", "x-user-defined", "replacement", "unknown"]) {
    output.encodings[label] = result(() => {
      const decoder = new TextDecoder(label);
      return [decoder.encoding, decoder.fatal, decoder.ignoreBOM, decoder.decode(new Uint8Array([0x80, 0xff]))];
    });
  }
  output.decoderErrors = [
    result(() => new TextDecoder("utf8", { fatal: true }).decode(new Uint8Array([0xff]))),
    result(() => new TextDecoder().decode([65])),
    result(() => new TextDecoder().decode(null)),
    result(() => new TextDecoder("utf8", null)),
    result(() => new TextDecoder("utf8", true)),
    result(() => new TextDecoder().decode(undefined, null)),
    result(() => new TextDecoder().decode(undefined, true)),
    result(() => new TextEncoder().encode(Symbol())),
  ];
  const fatalDecoder = new TextDecoder("utf8", { fatal: true });
  output.fatalStreaming = [
    result(() => fatalDecoder.decode(new Uint8Array([0xe2]), { stream: true })),
    result(() => fatalDecoder.decode(new Uint8Array([0xff]), { stream: true })),
    result(() => fatalDecoder.decode(new Uint8Array([65]))),
    result(() => fatalDecoder.decode(new Uint8Array([66]))),
  ];
  output.fatalState = {};
  for (const [label, chunks] of [
    ["utf8", [[239,187,191,65], [255], [239,187,191,66]]],
    ["utf-16le", [[255,254,65,0], [0,216,0,0], [255,254,66,0]]],
    ["iso-2022-jp", [[27,36,66,36,34], [255], [36,36]]],
  ]) {
    const decoder = new TextDecoder(label, { fatal: true });
    output.fatalState[label] = chunks.map(bytes => result(() => decoder.decode(new Uint8Array(bytes), { stream: true })));
  }
  output.decoderArguments = [
    result(() => new TextDecoder("utf8", () => {}).encoding),
    result(() => new TextDecoder().decode(new Uint8Array([65]), () => {})),
    result(() => { const buffer = new ArrayBuffer(8); buffer.transfer(); return new TextDecoder().decode(buffer); }),
    result(() => { const buffer = new ArrayBuffer(8); const view = new Uint8Array(buffer); buffer.transfer(); return new TextDecoder().decode(view); }),
  ];
  const conversions = [];
  new TextDecoder({ toString() { conversions.push("label"); return "utf8"; } }, {
    get fatal() { conversions.push("fatal"); return false; },
    get ignoreBOM() { conversions.push("ignoreBOM"); return false; },
  });
  output.conversions = conversions;
  output.view = new TextDecoder().decode(new DataView(new Uint8Array([0, 65, 66, 0]).buffer, 1, 2));
  output.encode = ["", "hello", "€", "😀", "\ud800", "\udc00", "\ud800A"].map(value => [...new TextEncoder().encode(value)]);
  output.encodeInto = [];
  for (const source of ["€", "😀a", "a\ud800b"]) {
    for (const size of [0, 1, 2, 3, 4, 5]) {
      const backing = new Uint8Array(size + 2).fill(99);
      const destination = backing.subarray(1, size + 1);
      output.encodeInto.push({ size, ...new TextEncoder().encodeInto(source, destination), bytes: [...backing] });
    }
  }
  output.encodeErrors = [Uint8ClampedArray, Int8Array, DataView].map(Type => result(() => new TextEncoder().encodeInto("", new Type(new ArrayBuffer(0)))));
  output.base64 = {};
  for (const value of ["", "aGVsbG8=", " aG\tVs\nbG8=\r", "AB", "/w==", "A", "A===", "ab=c", "aGVsbG8===", "é", "\fYQ=="]) {
    output.base64[value] = result(() => atob(value));
  }
  output.btoa = ["", "hello", "\x00\xff", "€", "😀", "\ud800"].map(value => result(() => btoa(value)));
  output.randomErrors = [
    result(() => crypto.getRandomValues(new Float16Array(1))),
    result(() => crypto.getRandomValues(new Float32Array(1))),
    result(() => crypto.getRandomValues(new DataView(new ArrayBuffer(1)))),
    result(() => crypto.getRandomValues(new Uint8Array(65537))),
    result(() => crypto.getRandomValues(null)),
  ];
  output.randomTypes = [Int8Array, Uint8Array, Uint8ClampedArray, Int16Array, Uint16Array, Int32Array, Uint32Array, BigInt64Array, BigUint64Array].map(Type => {
    const value = new Type(0);
    return crypto.getRandomValues(value) === value;
  });
  output.exceptionCodes = ["toString", "constructor", "InvalidCharacterError", "TypeMismatchError", "QuotaExceededError"].map(name => new DOMException("", name).code);
  const bytes = new Uint8Array(34).fill(99);
  const view = bytes.subarray(1, 33);
  const originalRandom = Math.random;
  Math.random = () => { throw new Error("Insecure random source"); };
  output.random = {
    identity: crypto.getRandomValues(view) === view,
    boundaries: bytes[0] === 99 && bytes[33] === 99,
    filled: view.some(value => value !== 99),
  };
  Math.random = originalRandom;
  const tasks = [];
  waitUntil(Promise.resolve().then(() => tasks.push("imported")));
  ctx.waitUntil(Promise.resolve().then(() => tasks.push("context")));
  await Promise.resolve();
  output.tasks = tasks;
  const headers = new Headers([["X-Test", "one"], ["x-test", "two"]]);
  const request = new Request("https://example.test/path", { method: "post", body: "payload" });
  output.web = {
    headers: [...headers], header: headers.get("x-test"),
    method: request.method, text: await request.text(),
    url: new URL("/next", request.url).href,
  };
  const chunks = () => new ReadableStream({ start(controller) {
    controller.enqueue(new Uint8Array([0xe2]));
    controller.enqueue(new Uint8Array([0x82, 0xac, 0xff]));
    controller.close();
  } });
  const response = new Response(chunks(), { status: 201, headers });
  output.bodies = {
    status: response.status,
    text: await response.text(),
    bytes: [...new Uint8Array(await new Response(chunks()).arrayBuffer())],
    requestBytes: [...new Uint8Array(await new Request("https://example.test", { method: "POST", body: new Uint8Array([0, 255, 1, 0]).subarray(1, 3) }).arrayBuffer())],
    responseBytes: [...new Uint8Array(await new Response(new DataView(new Uint8Array([0, 255, 1, 0]).buffer, 1, 2)).arrayBuffer())],
  };
  return output;
}
