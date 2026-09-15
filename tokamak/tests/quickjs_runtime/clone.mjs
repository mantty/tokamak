function outcome(callback) {
  try { return callback(); }
  catch (error) { return { error: error.name, code: error.code ?? null }; }
}

export async function cloneContracts() {
  const result = {};
  result.aliases = [new Date(1234), /abc/g, new Uint8Array([1,2]), new DataView(new ArrayBuffer(2)), new Blob(["a"]), new File(["a"], "name")].map(value => outcome(() => {
    const copy = structuredClone([value, value]);
    return { alias: copy[0] === copy[1], different: copy[0] !== value, tag: Object.prototype.toString.call(copy[0]) };
  }));
  result.regexp = outcome(() => { const value = /a/gi; value.lastIndex = 3; const copy = structuredClone(value); return [copy.source, copy.flags, copy.lastIndex]; });
  result.shadowed = [new Uint8Array([1, 2]), new DataView(new Uint8Array([1, 2]).buffer), /test/gi, new Date(1), new Map([[1, 2]]), new Set([3])].map(value => outcome(() => {
    let reads = 0;
    for (const key of ["buffer", "byteLength", "byteOffset", "length", "source", "flags", "getTime", "entries", "values"]) {
      Object.defineProperty(value, key, { get() { reads++; throw new Error(key); } });
    }
    const copy = structuredClone(value);
    return { tag: Object.prototype.toString.call(copy), reads, data: ArrayBuffer.isView(copy) ? [...new Uint8Array(copy.buffer)]
      : copy instanceof RegExp ? [copy.source, copy.flags] : copy instanceof Date ? +copy : [...copy] };
  }));
  result.trackingViews = ["typed", "data"].flatMap(kind => [false, true].map(fixed => outcome(() => {
    const buffer = new ArrayBuffer(4, { maxByteLength: 8 });
    const view = kind === "typed" ? new Uint8Array(buffer, 0, fixed ? 4 : undefined) : new DataView(buffer, 0, fixed ? 4 : undefined);
    const copy = structuredClone(view);
    copy.buffer.resize(6);
    return { length: copy.byteLength, max: copy.buffer.maxByteLength };
  })));
  result.outOfBounds = outcome(() => {
    const buffer = new ArrayBuffer(4, { maxByteLength: 8 });
    const view = new Uint8Array(buffer, 2);
    buffer.resize(1);
    return structuredClone(view).byteLength;
  });
  result.options = [1, "x", true, null, undefined, () => {}, Symbol()].map(options => outcome(() => structuredClone(1, options)));
  result.transferGetter = outcome(() => {
    let reads = 0;
    structuredClone(1, { get transfer() { reads++; return []; } });
    return reads;
  });
  result.errorResponse = outcome(() => {
    const copy = structuredClone(Response.error());
    return { type: copy.type, status: copy.status, statusText: copy.statusText };
  });
  result.boxed = [new Number(2), new Boolean(true), new String("str"), Object(3n), Object(Symbol("x"))].map(value => outcome(() => {
    const copy = structuredClone(value);
    return [Object.prototype.toString.call(copy), String(copy.valueOf()), copy !== value];
  }));
  result.properties = outcome(() => {
    let getterCalls = 0;
    const input = Object.create(null);
    Object.defineProperties(input, {
      value: { enumerable: true, get() { getterCalls++; return 42; } },
      hidden: { value: "hidden" },
      __proto__: { enumerable: true, value: { polluted: true } },
    });
    Object.defineProperty(input, "__proto__", { enumerable: true, value: { polluted: true } });
    input[Symbol("private")] = () => {};
    const copy = structuredClone(input);
    return { keys: Object.keys(copy), prototype: Object.getPrototypeOf(copy) === Object.prototype, polluted: copy.polluted ?? null,
      ownProto: Object.hasOwn(copy, "__proto__"), getterCalls, descriptor: Object.getOwnPropertyDescriptor(copy, "value") };
  });
  result.sparse = outcome(() => {
    const input = new Array(4); input[2] = "x"; input.extra = "yes";
    const copy = structuredClone(input);
    return { length: copy.length, keys: Object.keys(copy), extra: copy.extra };
  });
  result.errors = [new TypeError("type", { cause: { code: 1 } }), new AggregateError([1,2], "many"), new DOMException("denied", "NotAllowedError")].map(error => outcome(() => {
    error.extra = 3;
    const copy = structuredClone([error, error]);
    return { alias: copy[0] === copy[1], name: copy[0].name, message: copy[0].message,
      cause: copy[0].cause, extra: copy[0].extra, errors: copy[0].errors,
      error: copy[0] instanceof Error, dom: copy[0] instanceof DOMException };
  }));
  result.rejected = [() => {}, Symbol("x"), new WeakMap(), new WeakSet(), Promise.resolve(), new URL("https://example.com"), new Headers(), new Request("https://example.com"), new Response("x")].map(value => outcome(() => Object.prototype.toString.call(structuredClone(value))));
  result.transfers = ["duplicate", "unlisted", "invalid", "failedClone"].map(kind => outcome(() => {
    const buffer = new ArrayBuffer(4);
    new Uint8Array(buffer)[0] = 42;
    const transfer = kind === "duplicate" ? [buffer, buffer] : kind === "invalid" ? [{}] : [buffer];
    const value = kind === "unlisted" ? {} : kind === "failedClone" ? { buffer, fn() {} } : { buffer };
    const clone = outcome(() => structuredClone(value, { transfer }));
    return { original: buffer.byteLength, cloned: clone.buffer?.byteLength, byte: clone.buffer && new Uint8Array(clone.buffer)[0], error: clone.error, code: clone.code };
  }));
  result.cycles = outcome(() => {
    const value = new Map(); const set = new Set([value]); value.set(value, set);
    const copy = structuredClone(value);
    return copy.get(copy).has(copy);
  });
  result.mutatingCollections = ["map", "set"].map(kind => outcome(() => {
    const collection = kind === "map" ? new Map() : new Set();
    const value = { get entry() { kind === "map" ? collection.set("late", 2) : collection.add(2); return 1; } };
    kind === "map" ? collection.set("first", value) : collection.add(value);
    const copy = structuredClone(collection);
    return { size: copy.size, values: [...copy.values()] };
  }));
  result.errorCause = outcome(() => {
    const error = new Error("cause");
    let calls = 0;
    Object.defineProperty(error, "cause", { enumerable: true, get() { return ++calls; } });
    const copy = structuredClone(error);
    return { calls, cause: copy.cause, enumerable: Object.getOwnPropertyDescriptor(copy, "cause").enumerable };
  });
  result.proxyPrototype = outcome(() => {
    let traps = 0;
    const prototype = new Proxy({}, { getPrototypeOf() { traps++; throw new Error("prototype trap"); } });
    const source = Object.assign(Object.create(prototype), { value: 42 });
    return { copy: outcome(() => structuredClone(source)), traps };
  });
  result.changedHostPrototype = [new Blob(["data"]), new Headers([["x", "y"]]), new URL("https://e"), new ReadableStream(), new Date(1)].map(value => outcome(() => {
    Object.setPrototypeOf(value, null);
    const copy = structuredClone(value);
    return { tag: Object.prototype.toString.call(copy) };
  }));
  result.forgedHostPrototype = [Blob, Headers, URL, ReadableStream].map(Type => outcome(() => structuredClone(Object.create(Type.prototype))));
  result.sharedBuffer = outcome(() => {
    const buffer = new SharedArrayBuffer(8);
    const copy = structuredClone([buffer, buffer, new Uint8Array(buffer)]);
    new Uint8Array(buffer)[0] = 17;
    copy[2][1] = 23;
    return { different: copy[0] !== buffer, alias: copy[0] === copy[1], view: copy[2].buffer === copy[0], bytes: [...new Uint8Array(copy[0])], original: [...new Uint8Array(buffer)] };
  });
  result.growableSharedBuffer = outcome(() => {
    const buffer = new SharedArrayBuffer(4, { maxByteLength: 16 });
    const copy = structuredClone(buffer);
    buffer.grow(8);
    const first = [buffer.byteLength, copy.byteLength];
    copy.grow(12);
    return { first, second: [buffer.byteLength, copy.byteLength], max: copy.maxByteLength, growable: copy.growable };
  });
  result.hostProperties = [new Blob(["x"]), new Headers(), new Request("https://e"), new Response(null)].map(value => outcome(() => {
    value.extra = "custom";
    const copy = structuredClone(value);
    return { tag: Object.prototype.toString.call(copy), extra: copy.extra };
  }));
  result.resizableBuffer = outcome(() => {
    const buffer = new ArrayBuffer(4, { maxByteLength: 8 });
    const copy = structuredClone(buffer);
    copy.resize(6);
    return { length: copy.byteLength, max: copy.maxByteLength, original: buffer.byteLength, resizable: copy.resizable };
  });
  result.proxies = outcome(() => {
    let traps = 0;
    const proxy = new Proxy({}, { get() { traps++; return undefined; }, getPrototypeOf() { traps++; return null; }, ownKeys() { traps++; return []; } });
    return { failure: outcome(() => structuredClone(proxy)), traps };
  });
  result.transferViews = ["typed", "data", "shared"].map(kind => outcome(() => {
    const buffer = kind === "shared" ? new SharedArrayBuffer(4) : new ArrayBuffer(4);
    const view = kind === "data" ? new DataView(buffer) : new Uint8Array(buffer);
    const copy = outcome(() => structuredClone(view, { transfer: [view] }));
    return { original: buffer.byteLength, length: copy.byteLength, error: copy.error, code: copy.code };
  }));
  result.cryptoKey = await (async () => {
    const key = await crypto.subtle.generateKey({ name: "AES-GCM", length: 128 }, false, ["encrypt", "decrypt"]);
    return outcome(() => { const copy = structuredClone([key,key]); return { key: copy[0] instanceof CryptoKey, alias: copy[0] === copy[1], different: copy[0] !== key, algorithm: copy[0].algorithm, extractable: copy[0].extractable, usages: copy[0].usages }; });
  })();
  return result;
}
