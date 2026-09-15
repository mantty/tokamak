import EventEmitter from "node:events";
import streams from "node:stream";
import processModule from "node:process";
import { env, waitUntil } from "cloudflare:workers";

function errorResult(error) {
  return { error: error.name, dom: error instanceof DOMException, code: error.code ?? null };
}

function result(callback) {
  try { return callback(); }
  catch (error) { return errorResult(error); }
}

async function asyncResult(callback) {
  try { return await callback(); }
  catch (error) { return errorResult(error); }
}

function detailedResult(callback) {
  try { return callback(); }
  catch (error) { return { ...errorResult(error), message: error.message }; }
}

async function detailedAsyncResult(callback) {
  try { return await callback(); }
  catch (error) { return { ...errorResult(error), message: error.message }; }
}

function describeHtmlRewriterValue(value, fields = []) {
  return {
    type: typeof value,
    tag: Object.prototype.toString.call(value),
    own: Object.getOwnPropertyNames(value).sort(),
    prototype: Object.getOwnPropertyNames(Object.getPrototypeOf(value)).sort(),
    fields: Object.fromEntries(fields.map(name => [name, result(() => value[name])])),
  };
}

function describeHtmlRewriterAttributes(attributes) {
  return {
    type: typeof attributes,
    tag: Object.prototype.toString.call(attributes),
    own: Object.getOwnPropertyNames(attributes).sort(),
    prototype: Object.getOwnPropertyNames(Object.getPrototypeOf(attributes)).sort(),
    length: result(() => attributes.length),
    entries: result(() => [...attributes]),
  };
}

export async function run(handlerEnv, ctx, constructors) {
  const output = {};
  const globalSurface = new Set();
  for (let object = globalThis; object; object = Object.getPrototypeOf(object)) {
    for (const name of Object.getOwnPropertyNames(object)) {
      if (!name.startsWith("__tokamak_") && name !== "WebAssembly") globalSurface.add(name);
    }
  }
  output.globalSurface = [...globalSurface].sort();
  const globalClasses = [
    "AbortController", "AbortSignal", "Blob", "Body", "ByteLengthQueuingStrategy", "Cache", "CacheStorage", "CloseEvent",
    "CompressionStream", "CountQueuingStrategy", "Crypto", "CryptoKey", "CustomEvent", "DOMException", "DecompressionStream",
    "ErrorEvent", "Event", "EventSource", "EventTarget", "ExtendableEvent", "FetchEvent", "File", "FixedLengthStream", "FormData",
    "HTMLRewriter", "Headers", "IdentityTransformStream", "MessageChannel", "MessageEvent", "MessagePort", "Navigator", "Performance",
    "PerformanceEntry", "PerformanceMark", "PerformanceMeasure", "PerformanceObserver", "PerformanceObserverEntryList", "PerformanceResourceTiming",
    "PromiseRejectionEvent", "ReadableByteStreamController", "ReadableStream", "ReadableStreamBYOBReader", "ReadableStreamBYOBRequest",
    "ReadableStreamDefaultController", "ReadableStreamDefaultReader", "Request", "Response", "ScheduledEvent", "SubtleCrypto", "TailEvent",
    "TextDecoder", "TextDecoderStream", "TextEncoder", "TextEncoderStream", "TraceEvent", "TransformStream", "TransformStreamDefaultController",
    "URL", "URLPattern", "URLSearchParams", "WebSocket", "WebSocketPair", "WebSocketRequestResponsePair", "WorkerGlobalScope",
    "WritableStream", "WritableStreamDefaultController", "WritableStreamDefaultWriter",
  ];
  output.globalClassSurfaces = Object.fromEntries(globalClasses.map(name => {
    const value = globalThis[name];
    return [name, {
      type: typeof value,
      proto: value && Object.getOwnPropertyNames(value.prototype ?? {}).filter(name => !name.startsWith("__")).sort(),
      static: value && Object.getOwnPropertyNames(value).sort(),
    }];
  }));
  const builtinNames = [
    "assert", "assert/strict", "async_hooks", "buffer", "console", "constants", "crypto",
    "diagnostics_channel", "dns", "dns/promises", "events", "fs", "fs/promises", "http",
    "https", "module", "net", "os", "path", "path/posix", "path/win32", "perf_hooks",
    "process", "punycode", "querystring", "stream", "stream/consumers", "stream/promises",
    "stream/web", "string_decoder", "sys", "timers", "timers/promises", "tls", "url", "util",
    "util/types", "zlib", "child_process", "cluster", "dgram", "domain", "http2", "inspector",
    "inspector/promises", "readline", "readline/promises", "repl", "sqlite", "test", "trace_events", "v8",
    "vm", "wasi", "worker_threads", "_http_agent", "_http_client", "_http_common", "_http_incoming",
    "_http_outgoing", "_http_server", "_stream_duplex", "_stream_passthrough", "_stream_readable",
    "_stream_transform", "_stream_wrap", "_stream_writable", "_tls_common", "_tls_wrap", "tty",
  ];
  const builtinRequirements = {
    "assert": ["ok", "strictEqual", "deepStrictEqual"],
    "assert/strict": ["ok", "strictEqual", "deepStrictEqual"],
    "async_hooks": ["AsyncLocalStorage", "AsyncResource"],
    "buffer": ["Buffer", "Blob", "File"],
    "crypto": ["createHash", "randomBytes", "randomUUID", "subtle"],
    "diagnostics_channel": ["channel", "subscribe", "unsubscribe"],
    "dns": ["lookup", "promises"],
    "dns/promises": ["lookup"],
    "events": ["EventEmitter", "once", "on"],
    "fs": ["readFileSync", "writeFileSync", "promises"],
    "fs/promises": ["readFile", "writeFile"],
    "http": ["request", "get", "createServer"],
    "https": ["request", "get"],
    "module": ["builtinModules", "isBuiltin", "createRequire"],
    "net": ["connect", "createConnection"],
    "os": ["platform", "arch", "tmpdir"],
    "path": ["join", "resolve", "basename", "posix", "win32"],
    "path/posix": ["join", "resolve"],
    "path/win32": ["join", "resolve"],
    "perf_hooks": ["performance", "PerformanceObserver"],
    "process": ["env", "nextTick", "getBuiltinModule"],
    "querystring": ["parse", "stringify"],
    "stream": ["Readable", "Writable", "pipeline"],
    "stream/consumers": ["text", "json", "buffer"],
    "stream/promises": ["pipeline", "finished"],
    "stream/web": ["ReadableStream", "WritableStream", "TransformStream"],
    "string_decoder": ["StringDecoder"],
    "timers": ["setTimeout", "setImmediate"],
    "timers/promises": ["setTimeout", "setImmediate"],
    "tls": ["connect", "createSecureContext"],
    "url": ["URL", "URLSearchParams", "parse"],
    "util": ["format", "inspect", "promisify"],
    "util/types": ["isArrayBuffer", "isTypedArray"],
    "zlib": ["gzip", "gunzip", "createGzip"],
    "_http_agent": ["Agent", "globalAgent"],
    "_http_client": ["ClientRequest"],
    "_http_common": ["CRLF", "methods"],
    "_http_incoming": ["IncomingMessage"],
    "_http_outgoing": ["OutgoingMessage", "validateHeaderName"],
    "_http_server": ["STATUS_CODES", "Server", "ServerResponse"],
    "_stream_duplex": ["from", "fromWeb", "toWeb"],
    "_stream_passthrough": [],
    "_stream_readable": ["ReadableState", "_fromList", "from", "fromWeb", "toWeb", "wrap"],
    "_stream_transform": [],
    "_stream_wrap": [],
    "_stream_writable": ["WritableState", "fromWeb", "toWeb"],
    "_tls_common": ["SecureContext", "createSecureContext"],
    "_tls_wrap": ["TLSSocket", "connect"],
    "tty": ["ReadStream", "WriteStream", "isatty"],
  };
  output.builtins = Object.fromEntries(builtinNames.map(name => {
    const value = process.getBuiltinModule("node:" + name);
    const required = builtinRequirements[name] ?? [];
    return [name, {
      available: value !== undefined,
      kind: typeof value,
      required: value !== undefined && required.every(key => key in value),
    }];
  }));
  output.builtinSurfaces = Object.fromEntries([
    "_http_agent", "_http_client", "_http_common", "_http_incoming", "_http_outgoing", "_http_server",
    "_stream_duplex", "_stream_passthrough", "_stream_readable", "_stream_transform", "_stream_wrap",
    "_stream_writable", "_tls_common", "_tls_wrap", "tty",
  ].map(name => [name, Object.keys(process.getBuiltinModule("node:" + name)).sort()]));
  output.nodeBuiltinSurfaces = Object.fromEntries(builtinNames.map(name => [
    name,
    Object.keys(process.getBuiltinModule("node:" + name)).sort(),
  ]));
  output.cloudflareModules = {};
  for (const [name, module] of Object.entries({
    workers: await import("cloudflare:workers"),
    sockets: await import("cloudflare:sockets"),
    node: await import("cloudflare:node"),
  })) output.cloudflareModules[name] = Object.keys(module).sort();
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
  output.base64Missing = [result(() => atob()), result(() => btoa())];
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
  output.workerApis = {
    randomUUID: result(() => {
      const value = crypto.randomUUID();
      return /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(value);
    }),
    structuredClone: result(() => {
      const source = {
        date: new Date("2024-07-01T12:34:56Z"),
        map: new Map([["value", 42]]),
        set: new Set(["value"]),
        bytes: new Uint8Array([1, 2, 3]),
      };
      const clone = structuredClone(source);
      const view = new DataView(new Uint8Array([9, 8, 7, 6]).buffer, 1, 2);
      const viewClone = structuredClone(view);
      const transferable = new ArrayBuffer(2);
      new Uint8Array(transferable).set([4, 5]);
      const transferred = structuredClone(transferable, { transfer: [transferable] });
      return {
        distinct: clone !== source && clone.bytes !== source.bytes,
        date: clone.date.toISOString(),
        map: clone.map.get("value"),
        set: clone.set.has("value"),
        bytes: [...clone.bytes],
        view: { byteOffset: viewClone.byteOffset, byteLength: viewClone.byteLength, bytes: [...new Uint8Array(viewClone.buffer)] },
        transfer: { bytes: [...new Uint8Array(transferred)], detached: transferable.byteLength === 0 },
      };
    }),
    structuredCloneEdges: {
      error: result(() => {
        const source = new TypeError("boom", { cause: { value: 1 } });
        source.code = "ERR_TEST";
        const clone = structuredClone(source);
        return {
          tag: Object.prototype.toString.call(clone),
          name: clone.name,
          message: clone.message,
          cause: clone.cause,
          code: clone.code,
          stack: typeof clone.stack,
        };
      }),
      cycles: result(() => {
        const source = { value: 1, array: [] };
        source.self = source;
        source.array.push(source);
        const clone = structuredClone(source);
        return { self: clone.self === clone, array: clone.array[0] === clone };
      }),
      typedArray: result(() => {
        const buffer = new ArrayBuffer(6);
        new Uint8Array(buffer).set([1, 2, 3, 4, 5, 6]);
        const source = new Uint16Array(buffer, 2, 2);
        const clone = structuredClone(source);
        return { constructor: clone.constructor.name, offset: clone.byteOffset, length: clone.length, bytes: [...new Uint8Array(clone.buffer)] };
      }),
      transferDetached: result(() => {
        const buffer = new ArrayBuffer(2);
        const clone = structuredClone({ value: 1 }, { transfer: [buffer] });
        return { value: clone.value, detached: buffer.byteLength };
      }),
      invalidTransfer: [
        result(() => structuredClone({}, { transfer: null })),
        result(() => structuredClone({}, { transfer: [new Uint8Array()] })),
        result(() => { const buffer = new ArrayBuffer(1); return structuredClone({}, { transfer: [buffer, buffer] }); }),
      ],
      messagePort: result(() => structuredClone(new MessageChannel().port1)),
    },
    intl: result(() => ({
      date: new Intl.DateTimeFormat("en-GB", {
        dateStyle: "full", timeStyle: "long", timeZone: "Europe/London",
      }).format(new Date("2024-07-01T12:34:56Z")),
      dateParts: new Intl.DateTimeFormat("en-GB", {
        year: "numeric", month: "long", day: "numeric", timeZone: "UTC",
      }).formatToParts(new Date("2024-07-01T12:34:56Z")).map(({ type, value }) => [type, value]),
      number: new Intl.NumberFormat("en-GB", { style: "currency", currency: "GBP" }).format(1234.5),
      plural: new Intl.PluralRules("en-GB").select(2),
      locale: new Intl.Locale("en-GB").toString(),
    })),
    url: result(() => {
      const url = new URL("https://example.test/path?x=1&x=2");
      const params = url.searchParams;
      params.append("y", "3");
      const appended = { size: params.size, search: url.search, href: url.href };
      url.search = "?z=4";
      return { appended, replaced: [...url.searchParams], href: url.href };
    }),
    urlPattern: result(() => {
      const match = new URLPattern({ pathname: "/users/:id" }).exec("https://example.test/users/42");
      return match?.pathname.groups.id ?? null;
    }),
    abort: result(() => {
      const controller = new AbortController();
      const before = controller.signal.aborted;
      controller.abort("cancelled");
      return { before, after: controller.signal.aborted, reason: controller.signal.reason };
    }),
    abortEdges: {
      nullReason: (() => {
        const signal = AbortSignal.abort(null);
        return { aborted: signal.aborted, reason: signal.reason, reasonIsNull: signal.reason === null };
      })(),
      any: (() => {
        const first = AbortSignal.abort("first");
        const second = new AbortController();
        const signal = AbortSignal.any([second.signal, first]);
        return { aborted: signal.aborted, reason: signal.reason };
      })(),
      listener: (() => {
        const controller = new AbortController();
        const events = [];
        controller.signal.onabort = event => events.push([event.type, event.target === controller.signal]);
        controller.abort("reason");
        controller.signal.onabort = null;
        return { events, reason: controller.signal.reason };
      })(),
    },
    events: result(() => {
      const target = new EventTarget();
      const events = [];
      target.addEventListener("ready", () => events.push("first"), { once: true });
      target.addEventListener("ready", () => events.push("second"));
      target.dispatchEvent(new CustomEvent("ready", { detail: 42, cancelable: true }));
      target.dispatchEvent(new Event("ready"));
      return events;
    }),
    eventEdges: {
      objectAndSignal: result(() => {
        const target = new EventTarget();
        const events = [];
        const controller = new AbortController();
        return {
          add: result(() => { target.addEventListener("ready", event => events.push(["function", event.type]), { signal: controller.signal }); return true; }),
          object: result(() => { target.addEventListener("ready", { handleEvent: event => events.push(["object", event.type]) }); return true; }),
          first: result(() => target.dispatchEvent(new Event("ready"))),
          abort: result(() => { controller.abort(); return true; }),
          second: result(() => target.dispatchEvent(new Event("ready"))),
          booleanCapture: result(() => { target.addEventListener("ready", () => events.push(["capture", true]), true); return true; }),
          events,
        };
      }),
      cancelable: result(() => {
        const target = new EventTarget();
        const event = new Event("cancel", { cancelable: true });
        target.addEventListener("cancel", value => value.preventDefault());
        return { canceled: target.dispatchEvent(event), defaultPrevented: event.defaultPrevented };
      }),
      passive: result(() => {
        const target = new EventTarget();
        const event = new Event("passive", { cancelable: true });
        target.addEventListener("passive", value => value.preventDefault(), { passive: true });
        return { dispatched: target.dispatchEvent(event), defaultPrevented: event.defaultPrevented };
      }),
    },
    cryptoDigest: await asyncResult(async () => {
      const digest = new Uint8Array(await crypto.subtle.digest("SHA-256", new TextEncoder().encode("abc")));
      return [...digest].map(value => value.toString(16).padStart(2, "0")).join("");
    }),
    cryptoSubtle: await asyncResult(async () => {
      try {
        const key = await crypto.subtle.importKey(
          "raw", new TextEncoder().encode("secret"),
          { name: "HMAC", hash: "SHA-256" }, false, ["sign", "verify"],
        );
        const data = new TextEncoder().encode("message");
        const signature = new Uint8Array(await crypto.subtle.sign("HMAC", key, data));
        return {
          type: key.type,
          algorithm: key.algorithm,
          usages: key.usages,
          signature: [...signature].map(value => value.toString(16).padStart(2, "0")).join(""),
          length: signature.length,
          valid: await crypto.subtle.verify("HMAC", key, signature, data),
          objectForm: await crypto.subtle.verify({ name: "HMAC" }, key, signature, data),
          invalid: await crypto.subtle.verify("HMAC", key, signature.subarray(1), data),
        };
      } catch (error) {
        return { error: error.name, message: error.message, stack: String(error.stack) };
      }
    }),
    cryptoAesGcm: await asyncResult(async () => {
      const key = await crypto.subtle.importKey(
        "raw", new Uint8Array(16), { name: "AES-GCM" }, true, ["encrypt", "decrypt"],
      );
      const algorithm = { name: "AES-GCM", iv: new Uint8Array(12), additionalData: new Uint8Array([1, 2]) };
      const plaintext = new TextEncoder().encode("secret message");
      const ciphertext = new Uint8Array(await crypto.subtle.encrypt(algorithm, key, plaintext));
      const decrypted = new Uint8Array(await crypto.subtle.decrypt(algorithm, key, ciphertext));
      return {
        type: key.type,
        algorithm: key.algorithm,
        usages: key.usages,
        ciphertextLength: ciphertext.length,
        plaintext: new TextDecoder().decode(decrypted),
      };
    }),
    cryptoDigestStream: await asyncResult(async () => {
      const stream = new crypto.DigestStream("SHA-256");
      const writer = stream.getWriter();
      await writer.write(new Uint8Array([1, 2, 3]));
      const beforeClose = {
        bytesWritten: stream.bytesWritten.toString(),
        digestPromise: stream.digest instanceof Promise,
        writable: stream instanceof WritableStream,
        locked: stream.locked,
      };
      await writer.close();
      const digest = new Uint8Array(await stream.digest);
      return {
        beforeClose,
        afterCloseLocked: stream.locked,
        digest: [...digest].map(value => value.toString(16).padStart(2, "0")).join(""),
      };
    }),
    globals: {
      errorEvent: typeof ErrorEvent,
      closeEvent: typeof CloseEvent,
      defaultReader: typeof ReadableStreamDefaultReader,
      defaultController: typeof ReadableStreamDefaultController,
      byteController: typeof ReadableByteStreamController,
      byteRequest: typeof ReadableStreamBYOBRequest,
      writableController: typeof WritableStreamDefaultController,
      performance: typeof performance.now,
      timers: [setTimeout, clearTimeout, setInterval, clearInterval, setImmediate, clearImmediate].every(value => typeof value === "function"),
      cacheStorage: typeof CacheStorage,
      eventSource: typeof EventSource,
      htmlRewriter: typeof HTMLRewriter,
      messageChannel: typeof MessageChannel,
      navigator: typeof navigator,
    },
    navigator: {
      userAgent: navigator.userAgent,
      platform: navigator.platform,
      language: navigator.language,
      languages: [...navigator.languages],
      hardwareConcurrency: navigator.hardwareConcurrency,
      sendBeacon: typeof navigator.sendBeacon,
    },
    cache: {
      default: typeof caches.default,
      open: typeof caches.open,
      match: typeof caches.match,
      has: typeof caches.has,
      delete: typeof caches.delete,
      keys: typeof caches.keys,
    },
    htmlRewriter: await asyncResult(async () => {
      const response = new HTMLRewriter()
        .on("title", { element(element) { element.setInnerContent("rewritten"); } })
        .transform(new Response("<html><head><title>original</title></head></html>"));
      return await response.text();
    }),
    htmlRewriterProbe: await asyncResult(async () => {
      const events = [];
      const source = "<!doctype html>before<div ID='x'>Hello<!--comment--><span>world</span></div>after";
      const response = new HTMLRewriter()
        .onDocument({
          doctype(value) { events.push({ type: "document-doctype", value: describeHtmlRewriterValue(value, ["name", "publicId", "systemId"])}); },
          comments(value) { events.push({ type: "document-comment", value: describeHtmlRewriterValue(value, ["text"])}); },
          text(value) { events.push({ type: "document-text", value: describeHtmlRewriterValue(value, ["text", "lastInTextNode"])}); },
          end(value) { events.push({ type: "document-end", value: describeHtmlRewriterValue(value) }); },
        })
        .on("div", {
          element(value) {
            const descriptor = describeHtmlRewriterValue(value, ["tagName", "tagNamePreserveCase", "namespaceURI", "isSelfClosing", "canHaveContent"]);
            descriptor.attributes = describeHtmlRewriterAttributes(value.attributes);
            events.push({ type: "element", value: descriptor });
          },
          comments(value) { events.push({ type: "element-comment", value: describeHtmlRewriterValue(value, ["text"])}); },
          text(value) { events.push({ type: "element-text", value: describeHtmlRewriterValue(value, ["text", "lastInTextNode"])}); },
          end(value) { events.push({ type: "element-end", value: describeHtmlRewriterValue(value, ["name", "namePreserveCase"])}); },
        })
        .transform(new Response(source));
      return { output: await response.text(), events };
    }),
    htmlRewriterMutationProbe: await asyncResult(async () => {
      function describeReturn(value) {
        if (value === undefined) return "undefined";
        if (value === null || typeof value !== "object") return value;
        return Object.prototype.toString.call(value);
      }

      function record(returns, ...values) {
        returns.push(...values.map(describeReturn));
      }

      async function elementCase(callback, source = "<div id='x'>body</div>") {
        try {
          const returns = [];
          const response = new HTMLRewriter().on("div", {
            element(element) { callback(element, returns); },
          }).transform(new Response(source));
          return { output: await response.text(), returns };
        } catch (error) {
          return errorResult(error);
        }
      }

      const cases = {
        attributes: await elementCase((element, returns) => {
          record(returns, element.getAttribute("ID"), element.hasAttribute("id"));
          record(returns, element.setAttribute("data-x", "<&\""));
          record(returns, element.removeAttribute("ID"));
          record(returns, element.getAttribute("id"), element.hasAttribute("id"));
          returns.push([...element.attributes]);
        }),
        element: await elementCase((element, returns) => {
          record(returns, element.before("before"));
          record(returns, element.after("after"));
          record(returns, element.prepend("prepend"));
          record(returns, element.append("append"));
          record(returns, element.setInnerContent("inner"));
        }),
        contentTypes: await elementCase((element, returns) => {
          record(returns, element.setInnerContent("<b>html</b>", { html: true }));
        }),
        contentTypesFalse: await elementCase((element, returns) => {
          record(returns, element.setInnerContent("<b>text</b>", { html: false }));
        }),
        contentTypesNull: await elementCase((element, returns) => {
          record(returns, element.setInnerContent("<b>null</b>", null));
        }),
        removed: await elementCase((element, returns) => {
          record(returns, element.removed, element.remove(), element.removed);
        }),
        kept: await elementCase((element, returns) => {
          record(returns, element.removeAndKeepContent());
        }),
        tagNameProperty: await asyncResult(async () => {
          const response = new HTMLRewriter().on("div", {
            element(value) { value.tagName = "section"; },
          }).transform(new Response("<div>body</div>"));
          return await response.text();
        }),
        text: await asyncResult(async () => {
          const returns = [];
          const response = new HTMLRewriter().on("div", {
            text(value) {
              record(returns, value.text, value.lastInTextNode, value.removed);
              record(returns, value.before("before"), value.after("after"));
              record(returns, value.replace("<b>replacement</b>", { html: true }));
            },
          }).transform(new Response("<div>body</div>"));
          return { output: await response.text(), returns };
        }),
        textProperty: await asyncResult(async () => {
          const response = new HTMLRewriter().on("div", {
            text(value) { value.text = "changed"; },
          }).transform(new Response("<div>body</div>"));
          return await response.text();
        }),
        comment: await asyncResult(async () => {
          const returns = [];
          const response = new HTMLRewriter().on("div", {
            comments(value) {
              record(returns, value.text, value.removed);
              record(returns, value.before("before"), value.after("after"));
              record(returns, value.replace("replacement"));
            },
          }).transform(new Response("<div><!--comment--></div>"));
          return { output: await response.text(), returns };
        }),
        commentProperty: await asyncResult(async () => {
          const response = new HTMLRewriter().on("div", {
            comments(value) { value.text = "changed"; },
          }).transform(new Response("<div><!--comment--></div>"));
          return await response.text();
        }),
        endTag: await elementCase((element, returns) => {
          record(returns, element.onEndTag(end => {
            returns.push(describeHtmlRewriterValue(end, ["name", "namePreserveCase", "removed"]));
            record(returns, end.name, end.namePreserveCase);
          }));
        }),
        endTagMutation: await elementCase((element, returns) => {
          record(returns, element.onEndTag(end => end.before("before")));
        }),
        document: await asyncResult(async () => {
          const returns = [];
          const response = new HTMLRewriter().onDocument({
            text(value) { record(returns, value.replace("document")); },
            comments(value) { record(returns, value.replace("comment")); },
            end(value) { record(returns, value.append("end")); },
          }).transform(new Response("text<!--comment-->"));
          return { output: await response.text(), returns };
        }),
      };
      return cases;
    }),
    htmlRewriterResponseProbe: await asyncResult(async () => {
      const encoder = new TextEncoder();
      const input = new Response(new ReadableStream({
        start(controller) {
          controller.enqueue(encoder.encode("<di"));
          controller.enqueue(encoder.encode("v>body</div>"));
          controller.close();
        },
      }), {
        status: 201,
        statusText: "Created",
        headers: { "X-Test": "yes" },
      });
      const transformed = new HTMLRewriter().on("div", {
        element(element) { element.setInnerContent("changed"); },
      }).transform(input);
      const chunks = [];
      const reader = transformed.body.getReader();
      while (true) {
        const { done, value } = await reader.read();
        if (done) break;
        chunks.push(new TextDecoder().decode(value));
      }
      return {
        same: transformed === input,
        status: transformed.status,
        statusText: transformed.statusText,
        header: transformed.headers.get("x-test"),
        inputBodyUsed: input.bodyUsed,
        chunks,
        output: await new Response(new Blob(chunks)).text(),
      };
    }),
    htmlRewriterValidationProbe: {
      onReturn: result(() => {
        const writer = new HTMLRewriter();
        return writer.on("div", {}) === writer;
      }),
      missingHandlers: result(() => new HTMLRewriter().on("div")),
      documentReturn: result(() => {
        const writer = new HTMLRewriter();
        return writer.onDocument({}) === writer;
      }),
      invalidSelector: result(() => new HTMLRewriter().on(null, {})),
      invalidHandlers: result(() => new HTMLRewriter().on("div", null)),
      invalidDocument: result(() => new HTMLRewriter().onDocument(null)),
      invalidTransform: result(() => new HTMLRewriter().transform("html")),
    },
    htmlRewriterRegistrationProbe: await asyncResult(async () => {
      const calls = [];
      const writer = new HTMLRewriter()
        .onDocument({ end() { calls.push("first"); } })
        .onDocument({ end() { calls.push("second"); } })
        .on("div", { element() { calls.push("element-first"); } })
        .on("div", { element() { calls.push("element-second"); } });
      const response = writer.transform(new Response("<div></div>"));
      await response.text();
      return calls;
    }),
    htmlRewriterCallbackCaptureProbe: await asyncResult(async () => {
      const calls = [];
      const handlers = { element() { calls.push("original"); } };
      const writer = new HTMLRewriter().on("div", handlers);
      handlers.element = () => calls.push("changed");
      await writer.transform(new Response("<div></div>")).text();
      return calls;
    }),
    intlExtended: result(() => ({
      percent: new Intl.NumberFormat("en-GB", { style: "percent" }).format(0.56),
      list: new Intl.ListFormat("en-GB", { type: "disjunction" }).format(["a", "b", "c"]),
      relative: new Intl.RelativeTimeFormat("en-GB", { numeric: "auto" }).format(-1, "day"),
      compare: new Intl.Collator("en-GB").compare("a", "b") < 0,
      segments: [...new Intl.Segmenter("en", { granularity: "word" }).segment("hello world")].map(value => value.segment),
      maximize: new Intl.Locale("en").maximize().toString(),
    })),
    intlSurface: {
      date: result(() => {
        const formatter = new Intl.DateTimeFormat("en-GB", {
          weekday: "long", year: "numeric", month: "long", day: "numeric",
          hour: "2-digit", minute: "2-digit", second: "2-digit", timeZone: "Europe/London",
          timeZoneName: "short",
        });
        return {
          format: formatter.format(new Date("2024-07-01T12:34:56Z")),
          parts: formatter.formatToParts(new Date("2024-07-01T12:34:56Z")),
          options: formatter.resolvedOptions(),
        };
      }),
      number: result(() => {
        const formatter = new Intl.NumberFormat("de-DE", {
          style: "currency", currency: "EUR", currencyDisplay: "code", minimumFractionDigits: 2,
        });
        return {
          format: formatter.format(-1234.5),
          parts: formatter.formatToParts(-1234.5),
          options: formatter.resolvedOptions(),
        };
      }),
      plural: result(() => {
        const formatter = new Intl.PluralRules("ar", { type: "ordinal" });
        return {
          values: [0, 1, 2, 3, 11, 102].map(value => formatter.select(value)),
          range: formatter.selectRange(1, 2),
          options: formatter.resolvedOptions(),
        };
      }),
      list: result(() => {
        const formatter = new Intl.ListFormat("en-GB", { type: "unit", style: "short" });
        return { format: formatter.format(["1m", "2s", "3ms"]), parts: formatter.formatToParts(["1m", "2s", "3ms"]), options: formatter.resolvedOptions() };
      }),
      relative: result(() => {
        const formatter = new Intl.RelativeTimeFormat("en-GB", { numeric: "always", style: "short" });
        return { format: formatter.format(-2, "hour"), parts: formatter.formatToParts(-2, "hour"), options: formatter.resolvedOptions() };
      }),
      locale: result(() => {
        const locale = new Intl.Locale("en-US", { calendar: "gregory", hourCycle: "h12", numeric: true });
        return {
          string: locale.toString(), language: locale.language, region: locale.region, script: locale.script,
          baseName: locale.baseName, calendar: locale.calendar, calendars: locale.calendars,
          hourCycle: locale.hourCycle, hourCycles: locale.hourCycles, numberingSystem: locale.numberingSystem,
          numberingSystems: locale.numberingSystems, textInfo: locale.getTextInfo(), weekInfo: locale.getWeekInfo(),
          maximize: locale.maximize().toString(), minimize: locale.minimize().toString(),
        };
      }),
      displayNames: result(() => {
        const names = new Intl.DisplayNames("en", { type: "language" });
        return { language: names.of("fr"), region: new Intl.DisplayNames("en", { type: "region" }).of("GB"), options: names.resolvedOptions() };
      }),
      collator: result(() => {
        const collator = new Intl.Collator("en", { sensitivity: "base", numeric: true });
        return { order: ["2", "10", "1"].sort(collator.compare), equal: collator.compare("a", "A"), options: collator.resolvedOptions() };
      }),
      segmenter: result(() => {
        const segments = [...new Intl.Segmenter("en", { granularity: "word" }).segment("Hello, world!")];
        return { values: segments, options: new Intl.Segmenter("en", { granularity: "word" }).resolvedOptions() };
      }),
      supported: result(() => ({
        calendars: Intl.supportedValuesOf("calendar").slice(0, 3),
        numberingSystems: Intl.supportedValuesOf("numberingSystem").slice(0, 3),
        timeZones: Intl.supportedValuesOf("timeZone").filter(value => ["UTC", "Europe/London", "America/New_York"].includes(value)),
      })),
      additional: result(() => ({
        dateStyles: [
          new Intl.DateTimeFormat("en-GB", { dateStyle: "full", timeZone: "UTC" }).format(new Date("2024-07-01T12:34:56Z")),
          new Intl.DateTimeFormat("en-GB", { timeStyle: "long", timeZone: "UTC" }).format(new Date("2024-07-01T12:34:56Z")),
          new Intl.DateTimeFormat("en-GB", { hour: "numeric", hour12: true, timeZone: "UTC" }).format(new Date("2024-07-01T12:34:56Z")),
        ],
        numbers: [
          new Intl.NumberFormat("de-DE", { style: "percent" }).format(-1234.5),
          new Intl.NumberFormat("de-DE", { notation: "scientific" }).format(-1234.5),
          new Intl.NumberFormat("de-DE", { notation: "compact" }).format(12345),
          new Intl.NumberFormat("de-DE", { style: "unit", unit: "kilometer-per-hour", unitDisplay: "long" }).format(-1234.5),
          new Intl.NumberFormat("en-US", { signDisplay: "always" }).format(0),
        ],
        plural: {
          cardinal: new Intl.PluralRules("ru").resolvedOptions(),
          ordinal: new Intl.PluralRules("en", { type: "ordinal" }).resolvedOptions(),
          categories: new Intl.PluralRules("ru").selectRange(1, 2),
        },
        list: [
          new Intl.ListFormat("en", { type: "conjunction" }).format([]),
          new Intl.ListFormat("en", { type: "conjunction" }).format(["a"]),
          new Intl.ListFormat("en", { type: "conjunction" }).format(["a", "b"]),
          new Intl.ListFormat("en", { type: "conjunction" }).formatToParts(["a", "b"]),
        ],
        relative: [
          new Intl.RelativeTimeFormat("en", { numeric: "auto" }).format(-1, "day"),
          new Intl.RelativeTimeFormat("en", { style: "narrow" }).formatToParts(2, "hour"),
        ],
        locale: {
          language: new Intl.Locale("zh-Hant-TW").language,
          script: new Intl.Locale("zh-Hant-TW").script,
          region: new Intl.Locale("zh-Hant-TW").region,
          baseName: new Intl.Locale("en-US-u-ca-gregory-hc-h12-kn").baseName,
          textInfo: new Intl.Locale("ar").getTextInfo(),
          weekInfo: new Intl.Locale("en-US").getWeekInfo(),
        },
        displayNames: {
          calendar: new Intl.DisplayNames("en", { type: "calendar" }).of("gregory"),
          currency: new Intl.DisplayNames("en", { type: "currency" }).of("GBP"),
          dateTimeField: new Intl.DisplayNames("en", { type: "dateTimeField" }).of("month"),
          script: new Intl.DisplayNames("en", { type: "script" }).of("Latn"),
        },
        collator: [
          new Intl.Collator("en", { sensitivity: "base" }).compare("a", "A"),
          new Intl.Collator("en", { ignorePunctuation: true }).compare("a-b", "ab"),
        ],
        segmenter: {
          grapheme: [...new Intl.Segmenter("en", { granularity: "grapheme" }).segment("A\u030A🙂")].map(value => value.segment),
          sentence: [...new Intl.Segmenter("en", { granularity: "sentence" }).segment("One. Two!")].map(value => value.segment),
        },
      })),
    },
    timers: await asyncResult(async () => {
      const values = [];
      const canceled = setTimeout(() => values.push("canceled"), 0);
      clearTimeout(canceled);
      await new Promise(resolve => setTimeout(() => { values.push("timeout"); resolve(); }, 1));
      await new Promise(resolve => setImmediate(() => { values.push("immediate"); resolve(); }));
      const order = [];
      const long = new Promise(resolve => setTimeout(() => { order.push("long"); resolve(); }, 5));
      const short = new Promise(resolve => setTimeout(() => { order.push("short"); resolve(); }, 0));
      await Promise.all([short, long]);
      return { values, order };
    }),
    abortTimeout: await asyncResult(async () => {
      const signal = AbortSignal.timeout(1);
      await new Promise(resolve => setTimeout(resolve, 2));
      return { aborted: signal.aborted, reason: signal.reason?.name };
    }),
    cryptoAesErrors: await asyncResult(async () => {
      const key = await crypto.subtle.importKey(
        "raw", new Uint8Array(16), { name: "AES-GCM" }, false, ["encrypt", "decrypt"],
      );
      const algorithm = { name: "AES-GCM", iv: new Uint8Array(12) };
      const ciphertext = new Uint8Array(await crypto.subtle.encrypt(algorithm, key, new Uint8Array([1])));
      const failure = async input => {
        try { await crypto.subtle.decrypt(algorithm, key, input); return null; }
        catch (error) { return errorResult(error); }
      };
      ciphertext[0] ^= 1;
      return { tampered: await failure(ciphertext), short: await failure(new Uint8Array()) };
    }),
    cryptoDigestErrors: await asyncResult(async () => {
      try { await crypto.subtle.digest("SHA-224", new Uint8Array([1])); return null; }
      catch (error) { return errorResult(error); }
    }),
    cryptoKey: await asyncResult(async () => {
      const key = await crypto.subtle.importKey(
        "raw", new Uint8Array(16), { name: "AES-GCM" }, false, ["encrypt"],
      );
      const original = key.algorithm.name;
      try { key.algorithm.name = "HMAC"; } catch {}
      return {
        type: typeof CryptoKey,
        original,
        name: key.algorithm.name,
        constructor: typeof CryptoKey === "function" ? result(() => new CryptoKey()) : null,
      };
    }),
    timerMicrotasks: await asyncResult(async () => {
      const order = [];
      const first = new Promise(resolve => setTimeout(() => {
        order.push("first");
        queueMicrotask(() => order.push("micro"));
        resolve();
      }, 0));
      const second = new Promise(resolve => setTimeout(() => { order.push("second"); resolve(); }, 0));
      await Promise.all([first, second]);
      return order;
    }),
  };
  const nodeCrypto = process.getBuiltinModule("node:crypto");
  const nodeBufferCtor = process.getBuiltinModule("node:buffer").Buffer;
  output.nodeCrypto = {
    subtle: typeof nodeCrypto.subtle?.digest,
    hash: result(() => {
      const value = nodeCrypto.createHash("sha256").update("abc").digest();
      return { buffer: value instanceof nodeBufferCtor, hex: value.toString("hex") };
    }),
    hmac: result(() => nodeCrypto.createHmac("sha256", "secret").update("message").digest("hex")),
    randomLarge: result(() => nodeCrypto.randomBytes(65537).length),
  };
  output.nodeCryptoEdges = await asyncResult(async () => {
    const hash = nodeCrypto.createHash("sha256");
    const hmac = nodeCrypto.createHmac("sha256", "secret");
    const target = new Uint8Array(4);
    const filled = await new Promise(resolve => nodeCrypto.randomFill(target, (error, value) => resolve({ error: error ? errorResult(error) : null, length: value?.length ?? null })));
    const random = await new Promise(resolve => nodeCrypto.randomBytes(3, (error, value) => resolve({ error: error ? errorResult(error) : null, length: value?.length ?? null })));
    const randomInt = nodeCrypto.randomInt(10, 20);
    return {
      instances: {
        hash: hash instanceof nodeCrypto.Hash,
        hmac: hmac instanceof nodeCrypto.Hmac,
        key: result(() => new nodeCrypto.KeyObject()),
      },
      hash: hash.update("a").update("bc").digest("hex"),
      hmac: hmac.update("message").digest("hex"),
      hashOutput: {
        type: result(() => typeof nodeCrypto.hash("sha256", "abc")),
        value: result(() => String(nodeCrypto.hash("sha256", "abc"))),
        buffer: result(() => nodeCrypto.hash("sha256", "abc", { outputFormat: "buffer" }) instanceof nodeBufferCtor),
        hex: result(() => nodeCrypto.hash("sha256", "abc", { outputFormat: "hex" })),
        outputEncoding: result(() => ({ type: typeof nodeCrypto.hash("sha256", "abc", { outputEncoding: "buffer" }), buffer: nodeCrypto.hash("sha256", "abc", { outputEncoding: "buffer" }) instanceof nodeBufferCtor })),
      },
      filled: { error: filled.error, length: filled.length, changed: target.some(value => value !== 0) },
      random,
      randomInt: randomInt >= 10 && randomInt < 20,
      values: {
        fips: [nodeCrypto.fips, nodeCrypto.getFips()],
        hashes: nodeCrypto.getHashes().sort(),
        ciphers: nodeCrypto.getCiphers().sort(),
        curves: nodeCrypto.getCurves().sort(),
        constantKeys: Object.keys(nodeCrypto.constants).sort(),
        secureHeap: nodeCrypto.secureHeapUsed(),
      },
      timing: nodeCrypto.timingSafeEqual(new Uint8Array([1]), new Uint8Array([1])),
      prototypes: {
        hash: result(() => Object.getOwnPropertyNames(nodeCrypto.Hash.prototype).sort()),
        hmac: result(() => Object.getOwnPropertyNames(nodeCrypto.Hmac.prototype).sort()),
        randomUUID: result(() => /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(nodeCrypto.randomUUID())),
      },
      errors: [result(() => nodeCrypto.randomBytes(65537)), result(() => nodeCrypto.randomFill(new Uint8Array(1))), result(() => nodeCrypto.randomInt(1, 1)), result(() => nodeCrypto.timingSafeEqual(new Uint8Array(1), new Uint8Array(2)))],
      keyDerivation: result(() => {
        const key = nodeCrypto.createSecretKey(new Uint8Array([1, 2, 3]));
        return {
          key: {
            type: key.type,
            symmetricKeySize: key.symmetricKeySize,
            export: [...new Uint8Array(key.export())],
            equals: key.equals(nodeCrypto.createSecretKey(new Uint8Array([1, 2, 3]))),
          },
          hkdf: [...new Uint8Array(nodeCrypto.hkdfSync("sha256", "key", "salt", "info", 16))],
          pbkdf2: [...nodeCrypto.pbkdf2Sync("password", "salt", 1, 16, "sha256")],
        };
      }),
      keyObjects: result(() => {
        const { privateKey, publicKey } = nodeCrypto.generateKeyPairSync("rsa", { modulusLength: 1024 });
        const signature = nodeCrypto.createSign("sha256").update("message").sign(privateKey);
        const verified = nodeCrypto.createVerify("sha256").update("message").verify(publicKey, signature);
        const encrypted = nodeCrypto.publicEncrypt(publicKey, nodeBufferCtor.from("secret"));
        const decrypted = nodeCrypto.privateDecrypt(privateKey, encrypted);
        const cipher = nodeCrypto.createCipheriv("aes-256-gcm", nodeBufferCtor.alloc(32, 1), nodeBufferCtor.alloc(12, 2));
        const ciphertext = nodeBufferCtor.concat([cipher.update(nodeBufferCtor.from("secret")), cipher.final()]);
        const decipher = nodeCrypto.createDecipheriv("aes-256-gcm", nodeBufferCtor.alloc(32, 1), nodeBufferCtor.alloc(12, 2));
        decipher.setAuthTag(cipher.getAuthTag());
        const plaintext = nodeBufferCtor.concat([decipher.update(ciphertext), decipher.final()]);
        const parsed = nodeCrypto.createPrivateKey(privateKey.export({ format: "pem", type: "pkcs8" }));
        return {
          types: [privateKey.type, publicKey.type, privateKey.asymmetricKeyType, publicKey.asymmetricKeyType, parsed.type],
          verified,
          decrypted: decrypted.toString(),
          plaintext: plaintext.toString(),
          exports: [
            privateKey.export({ format: "pem", type: "pkcs8" }).includes("BEGIN PRIVATE KEY"),
            publicKey.export({ format: "pem", type: "spki" }).includes("BEGIN PUBLIC KEY"),
          ],
          generatedSecret: nodeCrypto.generateKeySync("aes", { length: 256 }).symmetricKeySize,
        };
      }),
      advanced: await asyncResult(async () => {
        const prime = nodeCrypto.generatePrimeSync(16);
        const alice = nodeCrypto.createECDH("prime256v1");
        const bob = nodeCrypto.createECDH("prime256v1");
        alice.generateKeys();
        bob.generateKeys();
        const aliceSecret = alice.computeSecret(bob.getPublicKey());
        const bobSecret = bob.computeSecret(alice.getPublicKey());
        const dhPrime = nodeBufferCtor.from("17", "hex");
        const dhAlice = nodeCrypto.createDiffieHellman(dhPrime, 2);
        const dhBob = nodeCrypto.createDiffieHellman(dhPrime, 2);
        dhAlice.generateKeys();
        dhBob.generateKeys();
        const { privateKey, publicKey } = nodeCrypto.generateKeyPairSync("rsa", { modulusLength: 1024 });
        const encrypted = nodeCrypto.privateEncrypt(privateKey, nodeBufferCtor.from("secret"));
        const info = nodeCrypto.getCipherInfo("aes-256-gcm");
        return {
          primes: {
            small: nodeCrypto.checkPrimeSync(nodeBufferCtor.from([0x0b])),
            composite: nodeCrypto.checkPrimeSync(nodeBufferCtor.from([0x0f])),
            generated: { type: prime.constructor.name, length: prime.byteLength, prime: nodeCrypto.checkPrimeSync(prime) },
            generatedBigint: typeof nodeCrypto.generatePrimeSync(16, { bigint: true }),
            async: await new Promise(resolve => nodeCrypto.checkPrime(nodeBufferCtor.from([0x0b]), (error, value) => resolve({ error: error ? error.code ?? error.name : null, value }))),
          },
          cipherInfo: {
            default: { name: info.name, nid: info.nid, blockSize: info.blockSize, ivLength: info.ivLength, keyLength: info.keyLength, mode: info.mode },
            options: nodeCrypto.getCipherInfo("aes-256-gcm", { keyLength: 32, ivLength: 12 })?.keyLength,
            unknown: nodeCrypto.getCipherInfo("unknown") === undefined,
          },
          ecdh: {
            equal: aliceSecret.equals(bobSecret),
            sizes: [alice.getPrivateKey().length, alice.getPublicKey().length, aliceSecret.length],
            compressedHex: alice.getPublicKey("hex", "compressed").length,
            converted: nodeCrypto.ECDH.convertKey(bob.getPublicKey(), "prime256v1", undefined, undefined, "compressed").length,
          },
          dh: { equal: dhAlice.computeSecret(dhBob.getPublicKey()).equals(dhBob.computeSecret(dhAlice.getPublicKey())), sizes: [dhAlice.getPrime().length, dhAlice.getGenerator().length] },
          scrypt: [...nodeCrypto.scryptSync("password", "salt", 16, { N: 16, r: 1, p: 1 })],
          rsaLegacy: nodeCrypto.publicDecrypt(publicKey, encrypted).toString(),
          flags: {
            engine: result(() => nodeCrypto.setEngine("")),
            fips: result(() => nodeCrypto.setFips(true)),
          },
        };
      }),
    };
  });
  output.bodyApis = {
    requestReader: await asyncResult(async () => {
      const request = new Request("https://example.test", {
        method: "POST", body: "payload", headers: { "content-type": "text/plain" },
      });
      const reader = request.body.getReader();
      const first = await reader.read();
      const second = await reader.read();
      return { first: [...first.value], done: second.done, bodyUsed: request.bodyUsed };
    }),
    formData: await asyncResult(async () => {
      const request = new Request("https://example.test", {
        method: "POST", body: "first=one&first=two&empty=", headers: {
          "content-type": "application/x-www-form-urlencoded;charset=UTF-8",
        },
      });
      const form = await request.formData();
      return { first: form.getAll("first"), empty: form.get("empty"), size: [...form].length };
    }),
    blobFile: await asyncResult(async () => {
      const file = new File(["hello"], "hello.txt", { type: "text/plain", lastModified: 123 });
      return { name: file.name, type: file.type, size: file.size, text: await file.text(), modified: file.lastModified };
    }),
    multipart: await asyncResult(async () => {
      const body = new TextEncoder().encode(
        "--boundary\r\n" +
        "Content-Disposition: form-data; name=\"text\"\r\n\r\nvalue\r\n" +
        "--boundary\r\n" +
        "Content-Disposition: form-data; name=\"file\"; filename=\"data.bin\"\r\n" +
        "Content-Type: application/octet-stream\r\n\r\n",
      );
      const suffix = new TextEncoder().encode("\r\n--boundary--\r\n");
      const bytes = new Uint8Array(body.length + 3 + suffix.length);
      bytes.set(body);
      bytes.set([0, 255, 1], body.length);
      bytes.set(suffix, body.length + 3);
      const request = new Request("https://example.test", {
        method: "POST", body: bytes,
        headers: { "content-type": "multipart/form-data; boundary=boundary" },
      });
      const form = await request.formData();
      const file = form.get("file");
      return { text: form.get("text"), name: file.name, type: file.type, bytes: [...new Uint8Array(await file.arrayBuffer())] };
    }),
    clones: await asyncResult(async () => {
      const request = new Request("https://example.test", { method: "POST", body: "request" });
      const requestClone = request.clone();
      const response = new Response("response");
      const responseClone = response.clone();
      return { before: request.bodyUsed, request: await requestClone.text(), response: await responseClone.text(), original: await request.text(), after: request.bodyUsed };
    }),
  };
  output.cacheApis = await asyncResult(async () => {
    const request = new Request("https://example.test/cache?b=2&a=1");
    const cache = await detailedAsyncResult(() => caches.open("contract"));
    if (cache?.error) return { open: cache };
    const response = new Response("cached", { headers: { "x-cache": "yes" } });
    const put = await detailedAsyncResult(async () => { await cache.put(request, response); return "resolved"; });
    const ignoredSearch = await detailedAsyncResult(async () => { const value = await cache.match("https://example.test/cache?other=1", { ignoreSearch: true }); return value ? await value.text() : null; });
    const keys = await detailedAsyncResult(async () => (await cache.keys()).map(value => value.url));
    const added = await detailedAsyncResult(async () => { await cache.add("https://example.test/cache-add"); return "resolved"; });
    const storage = {
      has: await detailedAsyncResult(() => caches.has("contract")),
      keys: await detailedAsyncResult(() => caches.keys()),
      match: await detailedAsyncResult(async () => { const value = await caches.match(request); return value ? await value.text() : null; }),
      deleted: await detailedAsyncResult(() => caches.delete("contract")),
      hasAfterDelete: await detailedAsyncResult(() => caches.has("contract")),
    };
    const defaultCache = caches.default;
    const defaultPut = await detailedAsyncResult(async () => { await defaultCache.put(request, new Response("default-cached")); return "resolved"; });
    return {
      put,
      ignoredSearch,
      keys,
      added,
      default: { put: defaultPut },
      storage,
    };
  });
  output.runtimeSurface = {
    globals: {
      self: self === globalThis,
      origin,
      fetch: typeof fetch,
      reportError: typeof reportError,
      queueMicrotask: typeof queueMicrotask,
      constructors: [
        "Body", "Crypto", "SubtleCrypto", "FetchEvent", "ExtendableEvent", "FixedLengthStream",
        "IdentityTransformStream", "Navigator", "Performance", "PerformanceEntry", "PerformanceMark",
        "PerformanceMeasure", "PerformanceObserver", "PerformanceObserverEntryList", "PromiseRejectionEvent",
        "ScheduledEvent", "TailEvent", "TraceEvent", "WebSocketRequestResponsePair", "WorkerGlobalScope",
        "ServiceWorkerGlobalScope",
      ].map(name => [name, typeof globalThis[name]]),
    },
    body: {
      request: Request.prototype instanceof Body,
      response: Response.prototype instanceof Body,
      methods: Object.getOwnPropertyNames(Body.prototype).sort(),
      responseJson: Response.json({ value: 42 }).headers.get("content-type"),
      responseBytes: [...new Uint8Array(await new Response("bytes").bytes())],
      responseBlob: await new Response("blob", { headers: { "content-type": "text/plain" } }).blob().then(async value => ({ type: value.type, text: await value.text() })),
    },
    url: {
      static: [typeof URL.canParse, typeof URL.parse, typeof URL.createObjectURL, typeof URL.revokeObjectURL],
      searchParams: (() => {
        const url = new URL("https://example.test/path?x=1&x=2");
        url.searchParams.delete("x", "1");
        const afterDelete = { values: [...url.searchParams], size: url.searchParams.size, search: url.search };
        url.searchParams.sort();
        return { afterDelete, sorted: url.href };
      })(),
    },
    streams: {
      from: typeof ReadableStream.from,
      values: typeof ReadableStream.prototype.values,
      fixed: typeof FixedLengthStream,
      identity: typeof IdentityTransformStream,
    },
  };
  output.webPlatformSurface = {
    websocket: result(() => {
      const pair = new WebSocketPair();
      const client = pair[0];
      const server = pair[1];
      const properties = Object.fromEntries(["url", "readyState", "protocol", "extensions", "binaryType"].map(name => [name, client[name]]));
      server.accept();
      server.send("hello");
      return {
        properties,
        serverReady: server.readyState,
        static: Object.fromEntries(["CONNECTING", "OPEN", "CLOSING", "CLOSED", "READY_STATE_CONNECTING", "READY_STATE_OPEN", "READY_STATE_CLOSING", "READY_STATE_CLOSED"].map(name => [name, WebSocket[name]])),
        prototype: Object.getOwnPropertyNames(WebSocket.prototype).sort(),
      };
    }),
    performance: result(() => ({
      prototype: Object.getOwnPropertyNames(Performance.prototype).sort(),
      mark: (() => { const value = performance.mark("surface", { detail: { value: 1 } }); return { name: value.name, entryType: value.entryType, duration: value.duration, detail: value.detail }; })(),
      measure: (() => { const value = performance.measure("surface-measure", "surface"); return { name: value.name, entryType: value.entryType, duration: value.duration }; })(),
      eventCounts: typeof performance.eventCounts,
      nodeTiming: typeof performance.nodeTiming,
      timerify: typeof performance.timerify,
      resourceTiming: typeof performance.markResourceTiming,
      resourceTimingSurface: {
        prototype: Object.getOwnPropertyNames(PerformanceResourceTiming.prototype).sort(),
        constructor: result(() => new PerformanceResourceTiming("resource")),
      },
    })),
    eventSource: await asyncResult(async () => {
      const stream = new ReadableStream({
        start(controller) {
          controller.enqueue(new TextEncoder().encode(
            ": ping\nretry: 10\ndata: one\ndata: two\nid: 7\nevent: update\n\ndata: last\n\n",
          ));
          controller.close();
        },
      });
      const source = EventSource.from(stream);
      const events = [];
      source.onmessage = event => events.push({ type: event.type, data: event.data, id: event.lastEventId });
      source.addEventListener("update", event => events.push({ type: event.type, data: event.data, id: event.lastEventId }));
      await new Promise(resolve => setTimeout(resolve, 5));
      return {
        url: source.url,
        readyState: source.readyState,
        withCredentials: source.withCredentials,
        events,
        prototype: Object.getOwnPropertyNames(EventSource.prototype).sort(),
        static: Object.getOwnPropertyNames(EventSource).sort(),
      };
    }),
  };
  output.performanceObserver = await asyncResult(async () => {
    performance.clearMarks();
    performance.clearMeasures();
    const calls = [];
    const observer = new PerformanceObserver(list => calls.push(list.getEntries().map(entry => ({
      name: entry.name,
      type: entry.entryType,
    }))));
    observer.observe({ entryTypes: ["mark", "measure"] });
    performance.mark("observed");
    performance.measure("observed-measure", "observed");
    await new Promise(resolve => setTimeout(resolve, 0));
    const records = observer.takeRecords().map(entry => ({ name: entry.name, type: entry.entryType }));
    observer.disconnect();
    return { calls, records };
  });
  output.cloudflareSurface = await asyncResult(async () => {
    const workers = await import("cloudflare:workers");
    const sockets = await import("cloudflare:sockets");
    const replacement = { FLAG: "replacement" };
    const restored = workers.withEnv(replacement, () => workers.env.FLAG);
    const span = workers.tracing.startSpan("contract");
    const socket = sockets.connect({ hostname: "", port: 0 }, { secureTransport: "starttls" });
    const upgradedSocket = socket.startTls();
    socket.close().catch(() => {});
    upgradedSocket.close().catch(() => {});
    const failedSocket = sockets.connect({ hostname: "", port: 0 });
    const failedSocketLifecycle = {
      opened: await asyncResult(async () => await failedSocket.opened),
      closed: await asyncResult(async () => await failedSocket.closed),
      close: await asyncResult(async () => await failedSocket.close()),
    };
    return {
      cacheDefault: typeof workers.cache.default,
      cacheDefaultOpen: typeof workers.cache.default?.match,
      env: workers.env.FLAG,
      restored,
      envAfter: workers.env.FLAG,
      rpcTarget: typeof workers.RpcTarget,
      rpcStub: typeof workers.RpcStub,
      waitUntil: typeof workers.waitUntil,
      restore: typeof workers.restore,
      tracing: {
        prototype: Object.getOwnPropertyNames(Object.getPrototypeOf(workers.tracing)).sort(),
        spanPrototype: Object.getOwnPropertyNames(Object.getPrototypeOf(span)).sort(),
        isTraced: span.isTraced,
        setAttribute: span.setAttribute("key", "value") === span,
        setAttributes: span.setAttributes({ key: "value" }) === span,
        end: span.end(),
        active: workers.tracing.startActiveSpan("active", () => "value"),
        entered: workers.tracing.enterSpan("entered", () => "value"),
      },
      sockets: {
        prototype: Object.getOwnPropertyNames(Object.getPrototypeOf(socket)).sort(),
        secureTransport: [socket.secureTransport, upgradedSocket.secureTransport],
        upgraded: [socket.upgraded, upgradedSocket.upgraded],
        startTls: [typeof socket.startTls, typeof upgradedSocket.startTls],
        streams: [typeof socket.readable.getReader, typeof socket.writable.getWriter],
        close: [typeof socket.close(), typeof upgradedSocket.close()],
        failedSocketLifecycle,
      },
      context: {
        own: Object.getOwnPropertyNames(ctx).sort(),
        prototype: Object.getOwnPropertyNames(Object.getPrototypeOf(ctx)).sort(),
        waitUntil: result(() => ({
          type: typeof ctx.waitUntil(Promise.resolve()),
          private: "__waitUntil" in ctx,
        })),
        passThroughOnException: result(() => ({
          type: typeof ctx.passThroughOnException(),
        })),
      },
    };
  });
  output.cloudflareNode = await asyncResult(async () => {
    const node = await import("cloudflare:node");
    const http = await import("node:http");
    const describe = value => value && {
      type: typeof value,
      own: Object.getOwnPropertyNames(value).sort(),
      prototype: Object.getOwnPropertyNames(Object.getPrototypeOf(value)).sort(),
    };
    const rawRequest = node.handleAsNodeRequest(new Request("https://example.test"));
    const rawRequestDescription = {
      type: typeof rawRequest,
      tag: Object.prototype.toString.call(rawRequest),
      constructor: rawRequest?.constructor?.name,
      own: Object.getOwnPropertyNames(rawRequest ?? {}).sort(),
      prototype: Object.getOwnPropertyNames(Object.getPrototypeOf(rawRequest ?? {})).sort(),
    };
    const server = http.createServer((request, response) => {
      const chunks = [];
      request.on("data", chunk => chunks.push(new TextDecoder().decode(chunk)));
      request.on("end", () => {
        response.setHeader("x-node", "yes");
        response.write("node ");
        response.end(`${request.method} ${request.url} ${chunks.join("")} ${request.socket.localPort}`);
      });
    });
    server.listen(8787);
    const handler = node.httpServerHandler(server);
    const response = await Promise.race([
      Promise.resolve(handler.fetch(new Request("https://example.test/path?x=1", { method: "POST", body: "response" }))).then(async value => ({
        status: value.status,
        body: await value.text(),
        header: value.headers.get("x-node"),
      }), error => errorResult(error)),
      new Promise(resolve => setTimeout(() => resolve({ timeout: true }), 10)),
    ]);
    return {
      exports: Object.keys(node).sort(),
      functions: [node.httpServerHandler.length, node.handleAsNodeRequest.length],
      handlers: [
        result(() => describe(node.httpServerHandler(8787))),
        result(() => describe(node.httpServerHandler({ port: 8787 }))),
        result(() => node.httpServerHandler()),
      ],
      invalidRequest: rawRequestDescription,
      serverResponse: response,
      serverAddress: server.address(),
      client: await asyncResult(async () => {
        const originalFetch = globalThis.fetch;
        let requestDetails;
        globalThis.fetch = async (input, init) => {
          requestDetails = {
            input: String(input), method: init.method,
            bodyType: typeof init.body,
            bodyTag: Object.prototype.toString.call(init.body),
            body: await new Response(init.body).text(),
          };
          return new Response("client response", { status: 201, headers: { "x-client": "yes" } });
        };
        try {
          return await new Promise((resolve, reject) => {
            const client = http.request({
              protocol: "http:", hostname: "example.test", path: "/client?x=1", method: "POST",
              headers: { "x-request": "yes" },
            }, response => {
              const chunks = [];
              response.on("data", chunk => chunks.push(new TextDecoder().decode(chunk)));
              response.on("end", () => resolve({
                status: response.statusCode,
                body: chunks.join(""),
                header: response.headers["x-client"],
                request: requestDetails,
              }));
              response.on("error", reject);
            });
            client.on("error", reject);
            client.end("request body");
          });
        } finally {
          globalThis.fetch = originalFetch;
        }
      }),
    };
  });
  output.streamApis = {
    tee: await asyncResult(async () => {
      const source = new ReadableStream({ start(controller) { controller.enqueue(new Uint8Array([1, 2])); controller.close(); } });
      const [left, right] = source.tee();
      const read = async stream => [...(await stream.getReader().read()).value];
      return { left: await read(left), right: await read(right) };
    }),
    text: await asyncResult(async () => {
      const encoded = new TextEncoderStream();
      const writer = encoded.writable.getWriter();
      const text = new Response(encoded.readable).text();
      await writer.write("hello");
      await writer.close();
      return text;
    }),
    compression: await asyncResult(async () => {
      const input = new TextEncoder().encode("tokamak compression");
      const compressed = new CompressionStream("gzip");
      const writer = compressed.writable.getWriter();
      await writer.write(input);
      await writer.close();
      const bytes = new Uint8Array(await new Response(compressed.readable).arrayBuffer());
      const decompressed = new DecompressionStream("gzip");
      const decompressedWriter = decompressed.writable.getWriter();
      await decompressedWriter.write(bytes);
      await decompressedWriter.close();
      return { nonEmpty: bytes.length > 0, roundTrip: await new Response(decompressed.readable).text() };
    }),
  };
  // --- streams/events differential contract ---
  const settle = promise => Promise.race([
    promise,
    new Promise(resolve => setTimeout(() => resolve("pending"), 20)),
  ]);
  const probe = callback => Promise.race([
    asyncResult(callback),
    new Promise(resolve => setTimeout(() => resolve({ timeout: true }), 100)),
  ]);
  const nodeReadable = chunks => {
    const source = new streams.Readable({ read() {} });
    for (const chunk of chunks) source.push(chunk);
    source.push(null);
    return source;
  };
  const streamConsumers = process.getBuiltinModule("node:stream/consumers");
  const streamPromises = process.getBuiltinModule("node:stream/promises");
  output.nodeStreamCompat = {
    consumers: {
      text: await probe(() => streamConsumers.text(nodeReadable(["hello ", new Uint8Array([119, 111, 114, 108, 100])]))),
      json: await probe(() => streamConsumers.json(nodeReadable(['{"value":42}']))),
      buffer: await probe(async () => [...await streamConsumers.buffer(nodeReadable([new Uint8Array([1, 2]), new Uint8Array([3, 4])]))]),
      blob: await probe(async () => {
        const value = await streamConsumers.blob(nodeReadable(["a", new Uint8Array([98, 255])]));
        return { size: value.size, bytes: [...new Uint8Array(await value.arrayBuffer())] };
      }),
      arrayBuffer: await probe(async () => {
        const value = await streamConsumers.arrayBuffer(nodeReadable(["a", new Uint8Array([98, 255])]));
        return { byteLength: value.byteLength, bytes: [...new Uint8Array(value)] };
      }),
      webText: await probe(async () => {
        const source = new ReadableStream({
          start(controller) { controller.enqueue(new Uint8Array([104, 105])); controller.close(); },
        });
        const value = await streamConsumers.text(source);
        return { value, locked: source.locked };
      }),
    },
    promises: {
      pipeline: await probe(async () => {
        const received = [];
        const source = new streams.Readable({ read() {} });
        const destination = new streams.Writable({ write(_chunk, _encoding, callback) { callback(); } });
        const write = destination.write.bind(destination);
        destination.write = (chunk, encoding, callback) => {
          received.push(String(chunk));
          return write(chunk, encoding, callback);
        };
        const completion = streamPromises.pipeline(source, destination);
        queueMicrotask(() => { source.push("a"); source.push("b"); source.push(null); });
        await completion;
        return received;
      }),
      finished: await probe(async () => {
        const source = new streams.Readable({ read() {} });
        source.on("data", () => {});
        const completion = streamPromises.finished(source);
        source.push("done");
        source.push(null);
        await completion;
        return "undefined";
      }),
    },
  };
  output.streamsEventsContract = {
    readable: await probe(async () => {
      const source = new ReadableStream({ start(controller) { controller.enqueue("value"); } });
      const reader = source.getReader();
      const first = await reader.read();
      const locked = source.locked;
      reader.releaseLock();
      const releasedRead = await reader.read().catch(errorResult);
      const releasedClosed = await settle(reader.closed.catch(errorResult));
      const replacement = source.getReader();
      await replacement.cancel("done");
      return {
        first, locked, unlocked: !source.locked,
        prototype: Object.getPrototypeOf(reader) === ReadableStreamDefaultReader.prototype,
        releasedRead, releasedClosed,
      };
    }),
    writable: await probe(async () => {
      const sink = { values: [], write(value) { this.values.push(value); }, close() { this.values.push("close"); } };
      const stream = new WritableStream(sink);
      const writer = stream.getWriter();
      const before = { locked: stream.locked, desiredSize: writer.desiredSize, ready: await writer.ready.then(() => true) };
      await writer.write("one");
      writer.releaseLock();
      const releasedWrite = await writer.write("two").catch(errorResult);
      const releasedClosed = await settle(writer.closed.catch(errorResult));
      const replacement = stream.getWriter();
      await replacement.write("two");
      await replacement.close();
      return {
        before, after: !stream.locked, releasedWrite, releasedClosed,
        prototype: Object.getPrototypeOf(writer) === WritableStreamDefaultWriter.prototype,
        values: sink.values,
      };
    }),
    propagation: await probe(async () => {
      const readError = new Error("read");
      const errored = new ReadableStream({ start(controller) { controller.error(readError); } });
      const reader = errored.getReader();
      const read = await reader.read().catch(errorResult);
      const closed = await reader.closed.catch(errorResult);
      const canceled = [];
      const received = [];
      const source = new ReadableStream({
        start(controller) { controller.enqueue(1); controller.enqueue(2); },
        cancel(reason) { canceled.push(reason.message); },
      });
      const destination = new WritableStream({
        write(value) { received.push(value); if (value === 2) throw new Error("write"); },
        abort(reason) { canceled.push("abort:" + reason.message); },
      });
      const piped = await source.pipeTo(destination).then(() => null, error => error.message);
      return { read, closed, piped, received, canceled, sourceLocked: source.locked, destinationLocked: destination.locked };
    }),
    pipeThrough: await probe(async () => {
      const source = new ReadableStream({ start(controller) { controller.enqueue(1); controller.close(); } });
      const transform = new TransformStream({ transform(value, controller) { controller.enqueue(value + 1); } });
      const output = source.pipeThrough(transform);
      const reader = output.getReader();
      const value = await reader.read();
      const done = await reader.read();
      return { same: output === transform.readable, value, done, sourceLocked: source.locked };
    }),
    tee: await probe(async () => {
      const value = { value: 1 };
      const source = new ReadableStream({ start(controller) { controller.enqueue(value); controller.close(); } });
      const [left, right] = source.tee();
      const [leftResult, rightResult] = await Promise.all([
        left.getReader().read(), right.getReader().read(),
      ]);
      const reasons = [];
      const cancelSource = new ReadableStream({ cancel(reason) { reasons.push(Array.isArray(reason) ? reason : String(reason)); } });
      const [cancelLeft, cancelRight] = cancelSource.tee();
      await Promise.all([cancelLeft.cancel("left"), cancelRight.cancel("right")]);
      return {
        same: leftResult.value === rightResult.value,
        left: leftResult.value, right: rightResult.value,
        locked: source.locked, cancelReasons: reasons,
      };
    }),
    transform: await probe(async () => {
      const calls = [];
      const transformer = {
        start(controller) { calls.push(["start", this === transformer, controller instanceof TransformStreamDefaultController]); },
        transform(value, controller) { calls.push(["transform", this === transformer]); controller.enqueue(value + 1); },
        flush(controller) { calls.push(["flush", controller instanceof TransformStreamDefaultController]); },
      };
      const stream = new TransformStream(transformer);
      const writer = stream.writable.getWriter();
      const reader = stream.readable.getReader();
      const valuePromise = reader.read();
      await writer.write(1);
      const value = await valuePromise;
      const donePromise = reader.read();
      await writer.close();
      const done = await donePromise;
      return { calls, value, done, readable: stream.readable instanceof ReadableStream, writable: stream.writable instanceof WritableStream };
    }),
    transformStart: await probe(async () => {
      const calls = [];
      let release;
      const start = new Promise(resolve => { release = resolve; });
      const stream = new TransformStream({
        start() { calls.push("start"); return start; },
        transform(value, controller) { calls.push("transform"); controller.enqueue(value); },
      });
      const writer = stream.writable.getWriter();
      const reader = stream.readable.getReader();
      const write = settle(writer.write(1).then(() => "resolved", errorResult));
      await Promise.resolve();
      const before = [...calls];
      release?.();
      const result = await settle(reader.read().then(value => ({ done: value.done, value: value.value }), errorResult));
      return { before, after: calls, write: await write, result };
    }),
    transformErrors: await probe(async () => {
      const transform = new TransformStream({
        async transform() { throw new Error("transform"); },
      });
      const transformWriter = transform.writable.getWriter();
      const transformReader = transform.readable.getReader();
      const transformReadPromise = transformReader.read().then(value => ({ done: value.done, value: value.value }), errorResult);
      const transformWrite = await settle(transformWriter.write(1).then(() => "resolved", error => error.message));
      const transformRead = await settle(transformReadPromise);

      const flush = new TransformStream({
        async flush() { throw new Error("flush"); },
      });
      const flushWriter = flush.writable.getWriter();
      const flushReader = flush.readable.getReader();
      const flushReadPromise = flushReader.read().then(value => ({ done: value.done, value: value.value }), errorResult);
      const flushClose = await settle(flushWriter.close().then(() => "resolved", error => error.message));
      const flushRead = await settle(flushReadPromise);
      return { transformWrite, transformRead, flushClose, flushRead };
    }),
    events: await probe(async () => {
      const basic = result(() => {
        const target = new EventTarget();
        const calls = [];
        const objectListener = { handleEvent(event) { calls.push(["object", this === objectListener, event.currentTarget === target, event.eventPhase]); } };
        const functionListener = event => {
          calls.push(["function", event.target === target, event.currentTarget === target, event.eventPhase]);
          event.preventDefault();
        };
        target.addEventListener("ready", functionListener, { once: true });
        target.addEventListener("ready", objectListener);
        const event = new CustomEvent("ready", { cancelable: true, detail: 0 });
        const first = target.dispatchEvent(event);
        const after = {
          defaultPrevented: event.defaultPrevented, cancelBubble: event.cancelBubble,
          returnValue: event.returnValue, currentTarget: event.currentTarget, target: event.target === target,
        };
        const second = target.dispatchEvent(new Event("ready"));
        return {
          calls, first, second, after,
          detail: event.detail, prototype: Object.getPrototypeOf(event) === CustomEvent.prototype,
        };
      });
      const passive = result(() => {
        const target = new EventTarget();
        const event = new Event("passive", { cancelable: true });
        target.addEventListener("passive", value => value.preventDefault(), { passive: true });
        return [target.dispatchEvent(event), event.defaultPrevented];
      });
      const signal = result(() => {
        const target = new EventTarget();
        const abortController = new AbortController();
        let signaled = 0;
        target.addEventListener("signal", () => { signaled += 1; }, { signal: abortController.signal });
        abortController.abort();
        target.dispatchEvent(new Event("signal"));
        return signaled;
      });
      return { constants: [Event.NONE, Event.CAPTURING_PHASE, Event.AT_TARGET, Event.BUBBLING_PHASE], basic, passive, signal };
    }),
    eventEmitter: await probe(async () => {
      const emitter = new EventEmitter();
      const calls = [];
      const regular = value => calls.push(["regular", value]);
      emitter.on("value", regular);
      emitter.prependOnceListener("value", value => calls.push(["once", value]));
      const before = { max: emitter.getMaxListeners(), count: emitter.listenerCount("value") };
      emitter.setMaxListeners(3);
      emitter.emit("value", 1);
      emitter.emit("value", 2);
      return {
        calls, before, after: { max: emitter.getMaxListeners(), count: EventEmitter.listenerCount(emitter, "value") },
        listeners: emitter.listeners("value").length, names: emitter.eventNames(),
        constants: { defaultMaxListeners: EventEmitter.defaultMaxListeners, errorMonitor: typeof EventEmitter.errorMonitor },
      };
    }),
    releaseTerminal: await probe(async () => {
      const outcome = promise => promise.then(() => "resolved", errorResult);
      const readable = new ReadableStream({ start(controller) { controller.close(); } });
      const reader = readable.getReader();
      await reader.read();
      const readableBefore = await outcome(reader.closed);
      reader.releaseLock();
      const readableAfter = await outcome(reader.closed);

      const writable = new WritableStream({ close() {} });
      const writer = writable.getWriter();
      await writer.close();
      const writableBefore = await outcome(writer.closed);
      writer.releaseLock();
      const writableAfter = await outcome(writer.closed);

      const open = new WritableStream();
      const openWriter = open.getWriter();
      const openReady = openWriter.ready.then(() => "resolved", errorResult);
      const openClosed = openWriter.closed.then(() => "resolved", errorResult);
      openWriter.releaseLock();
      return {
        readableBefore, readableAfter, writableBefore, writableAfter,
        openReady: await openReady, openClosed: await openClosed,
        releasedDesiredSize: result(() => openWriter.desiredSize),
        releasedReady: await settle(openWriter.ready.then(() => "resolved", errorResult)),
        releasedClosed: await settle(openWriter.closed.then(() => "resolved", errorResult)),
      };
    }),
    lockErrors: await probe(async () => {
      const readable = new ReadableStream();
      const reader = readable.getReader();
      const streamCancel = await asyncResult(() => readable.cancel("cancelled"));
      reader.releaseLock();
      const writable = new WritableStream();
      const writer = writable.getWriter();
      const streamAbort = await asyncResult(() => writable.abort("aborted"));
      writer.releaseLock();
      return { streamCancel, streamAbort };
    }),
    writableStart: await probe(async () => {
      const calls = [];
      let release;
      const start = new Promise(resolve => { release = resolve; });
      const stream = new WritableStream({
        start() { calls.push("start"); return start; },
        abort(reason) { calls.push(["abort", reason]); },
      });
      const abort = settle(stream.abort("aborted").then(() => "resolved", errorResult));
      await Promise.resolve();
      const before = [...calls];
      release?.();
      return { before, after: calls, abort: await abort };
    }),
    backpressure: await probe(async () => {
      const values = [];
      let release;
      const pending = new Promise(resolve => { release = resolve; });
      const stream = new WritableStream({ write(value) { values.push(value); return pending; } }, { highWaterMark: 1 });
      const writer = stream.getWriter();
      const write = writer.write("value");
      const during = { desiredSize: writer.desiredSize, ready: await settle(writer.ready.then(() => "resolved", errorResult)) };
      release?.();
      await write;
      return { during, after: { desiredSize: writer.desiredSize, ready: await settle(writer.ready.then(() => "resolved", errorResult)) }, values };
    }),
    closeReady: await probe(async () => {
      const stream = new WritableStream({ close() {} }, { highWaterMark: 0 });
      const writer = stream.getWriter();
      const ready = writer.ready.then(() => "resolved", errorResult);
      await writer.close();
      return { ready: await settle(ready), closed: await writer.closed.then(() => "resolved", errorResult) };
    }),
    prototypes: await probe(async () => ({
      readerRead: typeof ReadableStreamDefaultReader.prototype.read,
      readerRelease: typeof ReadableStreamDefaultReader.prototype.releaseLock,
      byobReadAtLeast: typeof ReadableStreamBYOBReader.prototype.readAtLeast,
      byteRequestGetter: typeof Object.getOwnPropertyDescriptor(ReadableByteStreamController.prototype, "byobRequest")?.get,
      byobViewGetter: typeof Object.getOwnPropertyDescriptor(ReadableStreamBYOBRequest.prototype, "view")?.get,
      byobAtLeastGetter: typeof Object.getOwnPropertyDescriptor(ReadableStreamBYOBRequest.prototype, "atLeast")?.get,
      writerWrite: typeof WritableStreamDefaultWriter.prototype.write,
      writableAbort: typeof WritableStream.prototype.abort,
      writableClose: typeof WritableStream.prototype.close,
      transformReadableGetter: typeof Object.getOwnPropertyDescriptor(TransformStream.prototype, "readable")?.get,
      transformWritableGetter: typeof Object.getOwnPropertyDescriptor(TransformStream.prototype, "writable")?.get,
      readerReadEnumerable: Object.getOwnPropertyDescriptor(ReadableStreamDefaultReader.prototype, "read")?.enumerable,
      byobReadAtLeastEnumerable: Object.getOwnPropertyDescriptor(ReadableStreamBYOBReader.prototype, "readAtLeast")?.enumerable,
      writableCloseEnumerable: Object.getOwnPropertyDescriptor(WritableStream.prototype, "close")?.enumerable,
      transformReadableEnumerable: Object.getOwnPropertyDescriptor(TransformStream.prototype, "readable")?.enumerable,
    })),
    construction: await probe(async () => ({
      defaultReader: result(() => new ReadableStreamDefaultReader()),
      byobReader: result(() => new ReadableStreamBYOBReader()),
      defaultController: result(() => new ReadableStreamDefaultController()),
      byteController: result(() => new ReadableByteStreamController()),
      byobRequest: result(() => new ReadableStreamBYOBRequest()),
      writableController: result(() => new WritableStreamDefaultController()),
      transformController: result(() => new TransformStreamDefaultController()),
    })),
    bytes: await probe(async () => {
      const run = async (method) => {
        let seen;
        const stream = new ReadableStream({
          type: "bytes",
          pull(controller) {
            const request = controller.byobRequest;
            seen = request && { atLeast: request.atLeast, length: request.view.byteLength, desiredSize: controller.desiredSize };
            request.view[0] = 7;
            request.respond(1);
          },
        });
        const reader = stream.getReader({ mode: "byob" });
        const view = new Uint8Array(4);
        const value = await reader[method](method === "readAtLeast" ? 1 : view, method === "readAtLeast" ? view : undefined);
        return { value: { done: value.done, bytes: [...value.value] }, seen, same: value.value === view };
      };
      const invalidReader = new ReadableStream({ type: "bytes" }).getReader({ mode: "byob" });
      const invalidView = new Uint8Array(1);
      const invalid = await Promise.all([
        settle(asyncResult(() => invalidReader.readAtLeast(0, invalidView))),
        settle(asyncResult(() => invalidReader.readAtLeast(1.5, invalidView))),
        settle(asyncResult(() => invalidReader.readAtLeast(2, invalidView))),
        settle(asyncResult(() => invalidReader.readAtLeast(1, null))),
      ]);
      invalidReader.releaseLock();
      return { read: await asyncResult(() => run("read")), readAtLeast: await asyncResult(() => run("readAtLeast")), invalid };
    }),
    pipeThrough: await probe(async () => {
      const error = new Error("read");
      const source = new ReadableStream({ start(controller) { controller.error(error); } });
      const transform = new TransformStream();
      const readable = source.pipeThrough(transform);
      const result = await readable.getReader().read().catch(errorResult);
      return { result, writable: await transform.writable.getWriter().closed.catch(errorResult) };
    }),
    transformCancel: await probe(async () => {
      const stream = new TransformStream({ cancel(reason) { return reason; } });
      const writer = stream.writable.getWriter();
      const closed = writer.closed.catch(errorResult);
      await stream.readable.cancel("cancelled");
      return { closed: await closed, writable: stream.writable.locked };
    }),
    eventGetters: await probe(async () => {
      const event = new Event("test", { bubbles: true, cancelable: true, composed: true });
      const before = {
        type: event.type, target: event.target, currentTarget: event.currentTarget,
        srcElement: event.srcElement, eventPhase: event.eventPhase, bubbles: event.bubbles,
        cancelable: event.cancelable, defaultPrevented: event.defaultPrevented,
        composed: event.composed, isTrusted: event.isTrusted,
        cancelBubble: event.cancelBubble, returnValue: event.returnValue,
        path: event.composedPath(),
      };
      event.cancelBubble = true;
      event.preventDefault();
      return {
        before,
        after: {
          cancelBubble: event.cancelBubble, defaultPrevented: event.defaultPrevented,
          returnValue: event.returnValue,
        },
        constants: [
          Event.NONE, Event.CAPTURING_PHASE, Event.AT_TARGET, Event.BUBBLING_PHASE,
          Event.prototype.NONE, Event.prototype.CAPTURING_PHASE,
        ],
      };
    }),
    listenerMutation: await probe(async () => {
      const target = new EventTarget();
      const calls = [];
      const added = () => calls.push("added");
      const first = () => {
        calls.push("first");
        target.addEventListener("change", added);
        target.removeEventListener("change", second);
      };
      const second = () => calls.push("second");
      target.addEventListener("change", first);
      target.addEventListener("change", second);
      target.dispatchEvent(new Event("change"));
      target.dispatchEvent(new Event("change"));
      return calls;
    }),
    eventDescriptors: await probe(async () => {
      const descriptor = name => {
        const value = Object.getOwnPropertyDescriptor(Event.prototype, name);
        return { get: typeof value?.get, set: typeof value?.set, enumerable: value?.enumerable };
      };
      return {
        type: descriptor("type"), target: descriptor("target"), currentTarget: descriptor("currentTarget"),
        cancelBubble: descriptor("cancelBubble"), returnValue: descriptor("returnValue"),
        preventDefault: descriptor("preventDefault"),
        listenerMethods: Object.fromEntries(["addEventListener", "removeEventListener", "dispatchEvent"].map(name => {
          const value = Object.getOwnPropertyDescriptor(EventTarget.prototype, name);
          return [name, { value: typeof value?.value, enumerable: value?.enumerable }];
        })),
        constants: Object.fromEntries(["NONE", "CAPTURING_PHASE", "AT_TARGET", "BUBBLING_PHASE"].map(name => [
          name, Object.getOwnPropertyDescriptor(Event.prototype, name),
        ])),
      };
    }),
  };
  output.messaging = await asyncResult(async () => {
    const channel = new MessageChannel();
    const received = [];
    channel.port2.onmessage = event => received.push(event.data.value);
    channel.port1.postMessage({ value: 42 });
    await Promise.resolve();
    return received;
  });
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

  // Fetch/URL contract probes.
  output.fetchUrlContracts = {
    bodyState: await asyncResult(async () => {
      const response = new Response("body");
      const initial = { body: response.body !== null, bodyUsed: response.bodyUsed, locked: response.body.locked };
      const reader = response.body.getReader();
      const locked = { bodyUsed: response.bodyUsed, locked: response.body.locked };
      const first = await reader.read();
      reader.releaseLock();
      return {
        initial, locked, first: [...first.value], afterRead: { bodyUsed: response.bodyUsed, locked: response.body.locked },
        cloneAfterRead: await asyncResult(async () => {
          const clone = response.clone();
          return { bodyUsed: clone.bodyUsed, text: await clone.text() };
        }),
        reuse: await asyncResult(() => response.text()),
      };
    }),
    bodyCancel: await asyncResult(async () => {
      const response = new Response("body");
      await response.body.cancel();
      return { bodyUsed: response.bodyUsed, locked: response.body.locked };
    }),
    streamClone: await asyncResult(async () => {
      const stream = new ReadableStream({ start(controller) { controller.enqueue(new Uint8Array([1, 2])); controller.close(); } });
      const response = new Response(stream);
      const clone = response.clone();
      const request = new Request("https://example.test", { method: "POST", body: new ReadableStream({ start(controller) { controller.enqueue(new Uint8Array([3])); controller.close(); } }) });
      const requestClone = request.clone();
      return {
        original: [...new Uint8Array(await response.arrayBuffer())],
        clone: [...new Uint8Array(await clone.arrayBuffer())],
        requestOriginalUsed: request.bodyUsed,
        requestCloneBody: requestClone.body !== null,
      };
    }),
    repeatedBody: await asyncResult(async () => {
      const response = new Response("body");
      return { first: await response.text(), second: await response.text(), bodyUsed: response.bodyUsed };
    }),
    requestState: await asyncResult(async () => {
      const request = new Request("https://example.test", { method: "POST", body: "body" });
      const copy = await asyncResult(() => new Request(request));
      const original = await asyncResult(() => request.text());
      const copyText = copy instanceof Request ? await asyncResult(() => copy.text()) : null;
      return { copy: copy instanceof Request, original, copyText, usedCopy: copy instanceof Request && copy.bodyUsed, usedOriginal: request.bodyUsed };
    }),
    nullBody: await asyncResult(async () => {
      const response = new Response();
      return { body: response.body, used: response.bodyUsed, text: await response.text(), usedAfter: response.bodyUsed };
    }),
    bodyInit: {
      urlSearchParams: new Request("https://example.test", { method: "POST", body: new URLSearchParams([["a", "b c"]]) }).headers.get("content-type"),
      blob: new Request("https://example.test", { method: "POST", body: new Blob(["x"], { type: "text/custom" }) }).headers.get("content-type"),
      formData: new Request("https://example.test", { method: "POST", body: new FormData() }).headers.get("content-type")?.startsWith("multipart/form-data; boundary="),
    },
    staticJsonUndefined: await asyncResult(async () => { const response = Response.json(undefined); return await response.text(); }),
    headers: {
      iterable: [...new Headers(new Map([["X-Test", "one"], ["x-test", "two"]]))],
      invalidName: result(() => new Headers([["bad name", "value"]])),
      invalidValue: result(() => new Headers([["x-test", "bad\nvalue"]])),
      setCookie: (() => { const headers = new Headers([["set-cookie", "a=1"], ["Set-Cookie", "b=2"]]); return { get: headers.get("set-cookie"), all: headers.getSetCookie(), entries: [...headers] }; })(),
    },
    url: result(() => {
      const params = new URLSearchParams([["space key", "!*'()~"], ["surrogate", "\ud800"]]);
      const url = new URL("https://example.test/path?x=1&x=2");
      url.searchParams.append("q", "a b");
      const appended = { search: url.search, href: url.href, size: url.searchParams.size };
      url.search = "?";
      return {
        encoded: params.toString(),
        percentLiteral: new URLSearchParams([["percent", "%20"]]).toString(),
        invalidPercent: [...new URLSearchParams("a=%")],
        appended,
        replaced: { search: url.search, values: [...url.searchParams], href: url.href },
        static: [URL.canParse("https://example.test"), URL.canParse("relative"), URL.parse("relative"), typeof URL.createObjectURL, typeof URL.revokeObjectURL],
        staticErrors: [result(() => URL.createObjectURL(new Blob(["x"]))), result(() => URL.createObjectURL({ size: 1 })), result(() => URL.revokeObjectURL("blob:x"))],
      };
    }),
    urlPattern: result(() => {
      const pattern = new URLPattern({ protocol: "https", hostname: "*.example.test", pathname: "/users/:id", search: "q=*" });
      const match = pattern.exec("https://api.example.test/users/42?q=ok");
      const regexp = new URLPattern({ pathname: "/(\\d+)" });
      const regexpMatch = regexp.exec("https://example.test/42");
      return { test: pattern.test("https://api.example.test/users/42?q=ok"), groups: match && { hostname: match.hostname.groups, id: match.pathname.groups.id, search: match.search.groups }, parts: match && { inputs: match.inputs, protocol: match.protocol, hostname: match.hostname, pathname: match.pathname, search: match.search, hash: match.hash }, regexp: { pathname: regexp.pathname, hasRegExpGroups: regexp.hasRegExpGroups, groups: regexpMatch?.pathname.groups } };
    }),
    errorsAndEdges: {
      relativeUrl: [result(() => new URL("relative")), result(() => new URL("/relative"))],
      paramsIterable: result(() => [...new URLSearchParams([new Set(["a", "b"])])]),
      paramsPrimitive: [new URLSearchParams(1).toString(), new URLSearchParams(true).toString(), result(() => new URLSearchParams(Symbol()).toString())],
      fileUrls: [result(() => new URL("file:///tmp/a").href), result(() => new URL("file:/tmp/a").href), result(() => new URL("file:c:/a").href), result(() => new URL("file://host/share").origin)],
      headerWhitespace: result(() => new Headers([["x-test", "  one\t two  "]]).get("x-test")),
      status: [result(() => new Response(null, { status: 0 })), result(() => new Response(null, { status: 1000 })), result(() => Response.redirect("https://example.test", 304))],
      statusConversions: [result(() => new Response(null, { status: 302.5 }).status), result(() => new Response(null, { status: null }).status), result(() => Response.redirect("https://example.test", 302.5).status)],
      blobSlice: result(() => { const blob = new Blob(["abcd"], { type: "TEXT/PLAIN" }).slice(3, 1, "TEXT/CUSTOM"); return { size: blob.size, type: blob.type }; }),
      blobOptions: [new Blob([], { type: null }).type, result(() => new File())],
      fileOptions: [new File([], "x", { type: null, lastModified: null }).type, new File([], "x", { lastModified: null }).lastModified],
      formSet: (() => { const form = new FormData(); form.append("a", "one"); form.append("b", "two"); form.append("a", "three"); form.set("a", "four"); return [...form]; })(),
      responseContentTypes: [
        new Response("text").headers.get("content-type"),
        new Response(new URLSearchParams("a=b")).headers.get("content-type"),
        new Response(new Blob(["x"], { type: "TEXT/X" })).headers.get("content-type"),
      ],
      requestDefaults: (() => { const request = new Request("https://example.test"); return { method: request.method, body: request.body, duplex: request.duplex, redirect: request.redirect, referrer: request.referrer }; })(),
      requestProperties: (() => { const request = new Request("https://example.test", { method: "POST", body: "body" }); return { cache: request.cache, credentials: request.credentials, destination: request.destination, integrity: request.integrity, keepalive: request.keepalive, mode: request.mode, referrer: request.referrer, referrerPolicy: request.referrerPolicy, duplex: request.duplex }; })(),
      requestValidation: {
        nullInit: result(() => new Request("https://example.test", null).body),
        redirect: [result(() => new Request("https://example.test", { redirect: "FOLLOW" }).redirect), result(() => new Request("https://example.test", { redirect: "invalid" }))],
        signal: [result(() => new Request("https://example.test", { signal: {} })), (() => { const first = new Request("https://example.test"); const second = new Request("https://example.test"); return { distinct: first.signal !== second.signal, aborted: first.signal.aborted }; })()],
        invalidCopy: (() => { const source = new Request("https://example.test", { method: "POST", body: "body" }); const outcome = result(() => new Request(source, { redirect: "invalid" })); return { outcome, used: source.bodyUsed }; })(),
        nullMembers: [result(() => new Request("https://example.test", { method: null })), result(() => new Request("https://example.test", { redirect: null })), result(() => new Request("https://example.test", { integrity: null }))],
      },
      responseValidation: { error: (() => { const response = Response.error(); const clone = response.clone(); return { status: response.status, ok: response.ok, type: response.type, clone: [clone.status, clone.ok, clone.type] }; })(), statusText: result(() => new Response(null, { statusText: "x\t" })) },
      requestSurface: (() => { const request = new Request("https://example.test"); return { own: Object.keys(request), prototype: Object.getOwnPropertyNames(Request.prototype).sort(), signal: typeof request.signal, cf: request.cf }; })(),
      responseSurface: (() => { const response = new Response("body"); return { own: Object.keys(response), prototype: Object.getOwnPropertyNames(Response.prototype).sort(), url: response.url, type: response.type, redirected: response.redirected, webSocket: response.webSocket }; })(),
      blobSurface: (() => { const blob = new Blob(["body"]); return { own: Object.keys(blob), prototype: Object.getOwnPropertyNames(Blob.prototype).sort(), filePrototype: Object.getOwnPropertyNames(File.prototype).sort(), formPrototype: Object.getOwnPropertyNames(FormData.prototype).sort(), headersPrototype: Object.getOwnPropertyNames(Headers.prototype).sort() }; })(),
      urlSurface: { url: Object.getOwnPropertyNames(URL.prototype).sort(), params: Object.getOwnPropertyNames(URLSearchParams.prototype).sort(), pattern: Object.getOwnPropertyNames(URLPattern.prototype).sort() },
    },
    formDataEdges: {
      malformedUrlEncoded: await asyncResult(async () => {
        const values = [];
        for (const body of ["a=%", "a=%GG", "a=%E2%82", "%GG=x"]) {
          const request = new Request("https://example.test", { method: "POST", body, headers: { "content-type": "application/x-www-form-urlencoded" } });
          values.push(await asyncResult(async () => [...await request.formData()]));
        }
        return values;
      }),
      emptyUrlEncoded: await asyncResult(async () => {
        const request = new Request("https://example.test", { method: "POST", body: "", headers: { "content-type": "application/x-www-form-urlencoded" } });
        return [...await request.formData()];
      }),
      quotedMultipart: await asyncResult(async () => {
        const body = new TextEncoder().encode("--b\r\nContent-Disposition: form-data; name=\"field\"\r\n\r\nvalue\r\n--b--\r\n");
        const request = new Request("https://example.test", { method: "POST", body, headers: { "content-type": 'multipart/form-data; boundary="b"' } });
        return [...await request.formData()];
      }),
      missingBoundary: await asyncResult(async () => {
        const request = new Request("https://example.test", { method: "POST", body: "body", headers: { "content-type": "multipart/form-data" } });
        return [...await request.formData()];
      }),
    },
  };
  const nodeAssert = process.getBuiltinModule("node:assert");
  const nodeAssertStrict = process.getBuiltinModule("node:assert/strict");
  const nodeBuffer = process.getBuiltinModule("node:buffer");
  const nodePath = process.getBuiltinModule("node:path");
  const nodeUrl = process.getBuiltinModule("node:url");
  const nodeQuerystring = process.getBuiltinModule("node:querystring");
  const nodeUtil = process.getBuiltinModule("node:util");
  const nodeModule = process.getBuiltinModule("node:module");
  const nodeTimers = process.getBuiltinModule("node:timers");
  const nodeTimersPromises = process.getBuiltinModule("node:timers/promises");
  const nodeOs = process.getBuiltinModule("node:os");
  const nodePerfHooks = process.getBuiltinModule("node:perf_hooks");
  const nodeDiagnostics = process.getBuiltinModule("node:diagnostics_channel");
  const nodeAsyncHooks = process.getBuiltinModule("node:async_hooks");
  const nodeStringDecoder = process.getBuiltinModule("node:string_decoder");
  const nodeConsole = process.getBuiltinModule("node:console");
  const nodeZlib = process.getBuiltinModule("node:zlib");
  const nodeNet = process.getBuiltinModule("node:net");
  const nodeTls = process.getBuiltinModule("node:tls");
  const nodeTest = process.getBuiltinModule("node:test");
  output.nodeCompat = {
    assert: {
      strictAlias: nodeAssert === nodeAssertStrict,
      equal: result(() => { nodeAssert.equal(1, "1"); return "ok"; }),
      deepMap: result(() => { nodeAssert.deepEqual(new Map([[{ a: 1 }, 2]]), new Map([[{ a: 1 }, 2]])); return "ok"; }),
      fail: result(() => nodeAssert.fail(["why"])),
      error: result(() => { throw new nodeAssert.AssertionError({ actual: 1, expected: 2, operator: "strictEqual", message: "custom" }); }),
    },
    buffer: (() => {
      const source = new Uint8Array([1, 2]);
      const shared = nodeBuffer.Buffer.from(source.buffer);
      shared[0] = 9;
      const copied = nodeBuffer.Buffer.from(source);
      copied[0] = 8;
      const slicedSource = nodeBuffer.Buffer.from([1, 2, 3]);
      slicedSource.slice(1)[0] = 9;
      const encoded = nodeBuffer.Buffer.from("deadbeef", "hex");
      return {
        constants: nodeBuffer.constants,
        encodings: [nodeBuffer.Buffer.from("abc", "hex"), nodeBuffer.Buffer.from("aGVsbG8===", "base64")].map(value => [...value]),
        shared: [source[0], [...shared]],
        copied: [source[0], [...copied]],
        slice: [...slicedSource],
        inspect: encoded.inspect(),
        json: encoded.toJSON(),
        static: [nodeBuffer.Buffer.byteLength("€"), nodeBuffer.Buffer.compare(nodeBuffer.Buffer.from([1]), nodeBuffer.Buffer.from([2])), nodeBuffer.Buffer.isEncoding("utf-8"), nodeBuffer.Buffer.of(1, 257)[1]],
        aliases: [nodeBuffer.Blob === Blob, nodeBuffer.File === File, nodeBuffer.atob === globalThis.atob, nodeBuffer.btoa === globalThis.btoa],
      };
    })(),
    path: {
      identity: [nodePath === nodePath.posix, nodePath.win32 !== nodePath.posix],
      posix: [nodePath.normalize("//a///b"), nodePath.join("/a", "", "b", "..", "c"), nodePath.resolve("a", "..", "b"), nodePath.dirname("/foo/bar"), nodePath.basename("/foo/bar.txt", ".txt"), nodePath.extname(".index.md"), nodePath.parse("/home/user/dir/file.txt"), nodePath.format({ dir: "/home/user/dir", name: "file", ext: ".txt" })],
      win32: [nodePath.win32.normalize("C:\\foo\\..\\bar"), nodePath.win32.join("C:\\a", "b", "..", "c"), nodePath.win32.resolve("C:foo"), nodePath.win32.relative("C:\\foo", "D:\\bar")],
      glob: [result(() => nodePath.matchesGlob("/a/b.txt", "/a/*.txt")), result(() => nodePath.win32.matchesGlob("C:\\a\\b.txt", "C:\\a\\*.txt"))],
    },
    url: (() => {
      const legacy = nodeUrl.parse("https://user:pass@example.com:8080/a%20b?x=1#h", true, true);
      const file = new nodeUrl.URL("file:///tmp/a");
      return {
        identity: [nodeUrl.URL === URL, nodeUrl.URLSearchParams === URLSearchParams, nodeUrl.parse !== nodeUrl.URL.parse],
        legacy: { protocol: legacy.protocol, slashes: legacy.slashes, auth: legacy.auth, host: legacy.host, hostname: legacy.hostname, port: legacy.port, pathname: legacy.pathname, search: legacy.search, query: legacy.query, hash: legacy.hash, path: legacy.path, href: legacy.href },
        format: nodeUrl.format({ protocol: "https:", host: "example.test", pathname: "/a b", query: { x: "1" } }),
        resolve: nodeUrl.resolve("https://example.test/a/b", "../c"),
        file: [nodeUrl.fileURLToPath(file), nodeUrl.pathToFileURL("relative/a").href, nodeUrl.toPathIfFileURL(file), nodeUrl.toPathIfFileURL("file:///tmp/a")],
        domains: [nodeUrl.domainToASCII("español.com"), nodeUrl.domainToUnicode("xn--espaol-zwa.com")],
        http: nodeUrl.urlToHttpOptions(new nodeUrl.URL("https://user:pass@example.com:8443/a?x=1#h")),
      };
    })(),
    querystring: (() => {
      const parsed = nodeQuerystring.parse("a+b=c+d&empty&x=");
      return { nullPrototype: Object.getPrototypeOf(parsed) === null, parsed, repeated: nodeQuerystring.parse("foo=bar&abc=xyz&abc=123"), stringified: nodeQuerystring.stringify({ foo: "bar", abc: ["xyz", "123"], empty: "", nil: null, undef: undefined, num: 42, bool: true, obj: { x: 1 } }), escaped: nodeQuerystring.escape("a b&c"), unescaped: nodeQuerystring.unescape("a+b%26c"), buffer: [...nodeQuerystring.unescapeBuffer("a+b%26c")], spaces: [...nodeQuerystring.unescapeBuffer("a+b", true)] };
    })(),
    util: {
      format: nodeUtil.format("%s:%d:%j:%o:%O:%%", "x", 2, { a: 1 }, { a: 1 }, { a: 1 }),
      inspect: [nodeUtil.inspect({ a: 1, b: "x", c: [true, null] }), nodeUtil.inspect({ a: { b: { c: 1 } } }, { depth: 1 }), nodeUtil.inspect(new Map([["a", 1]]))],
      predicates: [nodeUtil.isArray([]), nodeUtil.isBuffer(nodeBuffer.Buffer.from([1])), nodeUtil.isDeepStrictEqual({ a: 1 }, { a: 1 }), nodeUtil.types.isArrayBuffer(new ArrayBuffer(0)), nodeUtil.types.isTypedArray(new Uint8Array(0))],
      usv: nodeUtil.toUSVString("a\ud800b"),
      mime: (() => { const value = new nodeUtil.MIMEType("text/html; charset=utf-8"); return { essence: value.essence, type: value.type, subtype: value.subtype, params: [...value.params], string: value.toString() }; })(),
      parseArgs: result(() => nodeUtil.parseArgs()),
      parseEnv: result(() => nodeUtil.parseEnv("A=1\nB=two\n# comment\nEMPTY=\nQUOTED=\"a b\"")),
    },
    module: {
      aliases: [nodeModule === nodeModule.Module, nodeModule.builtinModules === nodeModule.Module.builtinModules, nodeModule.isBuiltin === nodeModule.Module.isBuiltin, nodeModule.createRequire === nodeModule.Module.createRequire, nodeModule === nodeModule.default],
      builtin: [nodeModule.isBuiltin("node:buffer"), nodeModule.isBuiltin("node:not-real")],
      require: nodeModule.createRequire("/bundle/contracts.mjs")("node:buffer") === nodeBuffer,
      stripTypeScriptTypes: result(() => nodeModule.stripTypeScriptTypes("const value: number = 1;")),
    },
    timers: await asyncResult(async () => {
      const value = await nodeTimersPromises.setTimeout(0, "value");
      const immediate = await nodeTimersPromises.setImmediate("immediate");
      const wait = await nodeTimersPromises.scheduler.wait(0);
      const controller = new AbortController();
      controller.abort();
      const aborted = await asyncResult(async () => nodeTimersPromises.setTimeout(0, "never", { signal: controller.signal }));
      return { identity: nodeTimers.promises === nodeTimersPromises, types: [typeof nodeTimers.setTimeout, typeof nodeTimers.setImmediate, typeof nodeTimersPromises.setInterval], values: [value, immediate, wait], aborted };
    }),
    os: {
      values: { EOL: nodeOs.EOL, devNull: nodeOs.devNull, arch: nodeOs.arch(), platform: nodeOs.platform(), type: nodeOs.type(), release: nodeOs.release(), version: nodeOs.version(), machine: nodeOs.machine(), endianness: nodeOs.endianness(), tmpdir: nodeOs.tmpdir(), homedir: nodeOs.homedir(), hostname: nodeOs.hostname(), availableParallelism: nodeOs.availableParallelism(), cpus: nodeOs.cpus(), loadavg: nodeOs.loadavg(), freemem: nodeOs.freemem(), totalmem: nodeOs.totalmem(), uptime: nodeOs.uptime(), networkInterfaces: nodeOs.networkInterfaces(), userInfo: nodeOs.userInfo() },
      constants: { udpReuse: nodeOs.constants.UV_UDP_REUSEADDR, priority: nodeOs.constants.priority, dlopen: nodeOs.constants.dlopen },
      priority: [nodeOs.getPriority(), result(() => nodeOs.setPriority(0))],
    },
    perfHooks: (() => {
      nodePerfHooks.performance.clearMarks();
      nodePerfHooks.performance.clearMeasures();
      const mark = nodePerfHooks.performance.mark("node-compat", { startTime: 1 });
      const measure = nodePerfHooks.performance.measure("node-compat-measure", "node-compat", "node-compat");
      return { identity: [nodePerfHooks.performance === performance, nodePerfHooks.PerformanceEntry === PerformanceEntry, nodePerfHooks.PerformanceObserver === PerformanceObserver], entries: [mark.name, mark.entryType, mark.startTime, measure.name, measure.entryType, measure.duration], supported: nodePerfHooks.PerformanceObserver.supportedEntryTypes, utilization: nodePerfHooks.eventLoopUtilization(), histogram: result(() => nodePerfHooks.createHistogram()) };
    })(),
    diagnostics: (() => {
      const value = nodeDiagnostics.channel("node-compat");
      const events = [];
      const listener = (message, name) => events.push([message.value, name]);
      value.subscribe(listener);
      const subscribed = value.hasSubscribers;
      value.publish({ value: 1 });
      value.unsubscribe(listener);
      const tracing = nodeDiagnostics.tracingChannel("node-compat-trace");
      return { subscribed, events, unsubscribed: value.hasSubscribers, tracing: Object.getOwnPropertyNames(Object.getPrototypeOf(tracing)).sort(), tracingSubscribers: tracing.hasSubscribers };
    })(),
    asyncHooks: (() => {
      const storage = new nodeAsyncHooks.AsyncLocalStorage({ name: "scope" });
      const scoped = storage.run({ value: 1 }, () => ({ inside: storage.getStore(), nested: storage.run(2, () => storage.getStore()), afterNested: storage.getStore() }));
      const resource = new nodeAsyncHooks.AsyncResource("resource");
      const hook = nodeAsyncHooks.createHook(null);
      return { storage: { name: storage.name, scoped, after: storage.getStore(), enterWith: result(() => storage.enterWith(1)), disable: result(() => storage.disable()) }, resource: { type: resource.type, asyncId: resource.asyncId(), triggerAsyncId: resource.triggerAsyncId(), scope: resource.runInAsyncScope(() => "ok") }, hook: hook.enable() === hook && hook.disable() === hook, ids: [nodeAsyncHooks.executionAsyncId(), nodeAsyncHooks.triggerAsyncId()], resourceType: typeof nodeAsyncHooks.executionAsyncResource(), providers: typeof nodeAsyncHooks.asyncWrapProviders.PROMISE };
    })(),
    stringDecoder: (() => {
      const decoder = new nodeStringDecoder.StringDecoder("utf8");
      return { encoding: decoder.encoding, chunks: [decoder.write(new Uint8Array([0xe2])), decoder.write(new Uint8Array([0x82, 0xac])), decoder.end()], state: [decoder.lastNeed, decoder.lastTotal, decoder.lastChar] };
    })(),
    streamEncoding: (() => {
      const readable = new streams.Readable();
      readable.setEncoding("utf8");
      const chunks = [];
      readable.on("data", value => chunks.push(value));
      readable.push(new Uint8Array([0xe2]));
      readable.push(new Uint8Array([0x82, 0xac]));
      readable.push(null);
      return chunks;
    })(),
    console: { identity: nodeConsole === globalThis.console, methods: ["log", "warn", "error", "time", "timeEnd", "table", "trace"].map(name => typeof nodeConsole[name]), constructor: result(() => new nodeConsole.Console()) },
    zlib: (() => {
      const compressed = nodeZlib.gzipSync("hello");
      const brotli = nodeZlib.brotliCompressSync("brotli");
      const zstd = nodeZlib.zstdCompressSync("zstd");
      return { roundTrip: nodeZlib.gunzipSync(compressed).toString(), compressed: [...compressed], brotli: nodeZlib.brotliDecompressSync(brotli).toString(), zstd: nodeZlib.zstdDecompressSync(zstd).toString(), constants: { noFlush: nodeZlib.constants.Z_NO_FLUSH, finish: nodeZlib.constants.Z_FINISH, defaultLevel: nodeZlib.constants.Z_DEFAULT_LEVEL, gzip: nodeZlib.constants.GZIP, deflate: nodeZlib.constants.DEFLATE, raw: nodeZlib.constants.DEFLATERAW, unzip: nodeZlib.constants.UNZIP }, codes: { ok: nodeZlib.codes.Z_OK, streamEnd: nodeZlib.codes.Z_STREAM_END, dataError: nodeZlib.codes.Z_DATA_ERROR }, transforms: [typeof nodeZlib.createGzip, typeof nodeZlib.Gzip], supportedSurface: [typeof nodeZlib.brotliCompressSync, typeof nodeZlib.zstdCompressSync] };
    })(),
    net: {
      functions: [typeof nodeNet.connect, typeof nodeNet.createConnection, typeof nodeNet.createServer],
      socket: result(() => {
        const socket = new nodeNet.Socket();
        return { prototype: Object.getOwnPropertyNames(Object.getPrototypeOf(socket)).sort(), destroyed: socket.destroyed };
      }),
      server: result(() => nodeNet.createServer()),
    },
    tls: {
      functions: [typeof nodeTls.connect, typeof nodeTls.createSecureContext, typeof nodeTls.createServer],
      context: result(() => ({ prototype: Object.getOwnPropertyNames(Object.getPrototypeOf(nodeTls.createSecureContext())).sort() })),
      socket: result(() => {
        const socket = new nodeTls.TLSSocket();
        return { prototype: Object.getOwnPropertyNames(Object.getPrototypeOf(socket)).sort(), encrypted: socket.encrypted };
      }),
    },
    nodeTest: {
      exports: result(() => Object.keys(nodeTest).sort()),
      types: result(() => ({
        mock: typeof nodeTest.mock,
        tracker: typeof nodeTest.MockTracker,
        context: typeof nodeTest.MockFunctionContext,
        fn: typeof nodeTest.mock.fn,
        method: typeof nodeTest.mock.method,
        getter: typeof nodeTest.mock.getter,
        setter: typeof nodeTest.mock.setter,
        reset: typeof nodeTest.mock.reset,
        restoreAll: typeof nodeTest.mock.restoreAll,
      })),
      trackerSurface: result(() => ({
        tracker: Object.getOwnPropertyNames(nodeTest.MockTracker.prototype).sort(),
        mock: Object.getOwnPropertyNames(Object.getPrototypeOf(nodeTest.mock)).sort(),
        context: Object.getOwnPropertyNames(nodeTest.MockFunctionContext.prototype).sort(),
      })),
      fn: result(() => {
        const tracker = new nodeTest.MockTracker();
        const seen = [];
        const fn = tracker.fn(function (...args) {
          seen.push(args.map(value => typeof value));
          return args.map(value => typeof value).join(":");
        });
        const receiver = { marker: true };
        const values = [fn.call(receiver, 1, "two"), fn(3)];
        const calls = fn.mock.calls.map(call => ({
          properties: Object.getOwnPropertyNames(call).sort(),
          arguments: call.arguments,
          thisValue: call.this,
        }));
        const copiedCalls = fn.mock.calls;
        copiedCalls.pop();
        const throwing = tracker.fn(() => { throw new Error("boom"); });
        const thrown = result(() => throwing());
        const thrownCall = throwing.mock.calls[0];
        function Constructor(value) { this.value = value; }
        const constructor = tracker.fn(Constructor);
        const instance = new constructor(5);
        const constructorCall = constructor.mock.calls[0];
        fn.mock.mockImplementation((...args) => `implementation:${args.join(",")}`);
        const implementation = fn("a");
        fn.mock.mockImplementationOnce((...args) => `once:${args.join(",")}`);
        const once = [fn("b"), fn("c")];
        fn.mock.resetCalls();
        const resetCalls = fn.mock.callCount();
        fn.mock.restore();
        return {
          values,
          seen,
          calls,
          surface: Object.getOwnPropertyNames(fn).sort(),
          copiedCalls: [copiedCalls.length, fn.mock.calls.length],
          thrown,
          thrownCall: { error: thrownCall.error?.message, result: thrownCall.result },
          constructor: { value: instance.value, target: typeof constructorCall.target, targetName: constructorCall.target?.name, thisValue: constructorCall.this?.value },
          implementation,
          once,
          resetCalls,
          restored: fn("d"),
        };
      }),
      methods: result(() => {
        const tracker = new nodeTest.MockTracker();
        const target = {
          value: 2,
          method(left, right) { return this.value + left + right; },
          get answer() { return this.value; },
          set answer(value) { this.value = value; },
        };
        const method = tracker.method(target, "method", (...args) => `method:${args.join(",")}`);
        const getter = tracker.getter(target, "answer", (...args) => `getter:${args.join(",")}`);
        const setter = tracker.setter(target, "answer", (...args) => `setter:${args.join(",")}`);
        target.answer = 9;
        return {
          method: target.method(3, 4),
          getter: target.answer,
          setterCalls: setter.mock.calls.map(call => call.arguments),
          methodCalls: method.mock.calls.map(call => call.arguments),
          getterCalls: getter.mock.calls.map(call => call.arguments),
        };
      }),
      resetRestore: result(() => {
        const tracker = new nodeTest.MockTracker();
        const target = { value: 1, method() { return "original"; } };
        const method = tracker.method(target, "method", () => "mocked");
        const before = target.method();
        tracker.reset();
        const afterReset = { callCount: method.mock.callCount(), value: target.method(), identity: target.method === method };
        tracker.restoreAll();
        return { before, afterReset, restored: target.method(), identity: target.method === method };
      }),
      contextRestore: result(() => {
        const tracker = new nodeTest.MockTracker();
        const target = { method() { return "original"; } };
        const method = tracker.method(target, "method", () => "mocked");
        method.mock.mockImplementation(() => "changed");
        const changed = target.method();
        method.mock.restore();
        return { changed, restored: target.method(), identity: target.method === method };
      }),
      options: result(() => {
        const tracker = new nodeTest.MockTracker();
        const fn = tracker.fn(() => "original", () => "mocked", { times: 2 });
        const values = [fn(), fn(), fn(), fn()];
        const once = tracker.fn(() => "original");
        once.mock.mockImplementationOnce(() => "next", 2);
        const onceValues = [once(), once(), once(), once()];
        return { values, onceValues, calls: [fn.mock.callCount(), once.mock.callCount()] };
      }),
      validation: {
        fn: [result(() => nodeTest.mock.fn(null)), result(() => nodeTest.mock.fn(() => {}, () => {}, { times: 0 })), result(() => nodeTest.mock.fn(() => {}, { times: null }))],
        method: [result(() => nodeTest.mock.method({}, "missing")), result(() => nodeTest.mock.method({ method: 1 }, "method"))],
        once: [
          result(() => { const fn = nodeTest.mock.fn(); fn(); return fn.mock.mockImplementationOnce(() => {}, 0); }),
          result(() => nodeTest.mock.fn().mock.mockImplementationOnce(() => {}, -1)),
          result(() => nodeTest.mock.fn().mock.mockImplementationOnce(() => {}, null)),
          result(() => nodeTest.mock.fn().mock.mockImplementationOnce(() => {}, 1.5)),
        ],
        context: result(() => {
          const context = new nodeTest.MockFunctionContext();
          return { callCount: context.callCount(), calls: context.calls };
        }),
      },
      global: result(() => {
        const target = { method() { return "original"; } };
        const fn = nodeTest.mock.fn(() => "mocked");
        nodeTest.mock.method(target, "method", () => "method");
        const before = [fn(), target.method()];
        nodeTest.mock.restoreAll();
        const restored = [fn(), target.method()];
        nodeTest.mock.reset();
        return { before, restored, identity: target.method.name };
      }),
    },
  };
  return output;
}
