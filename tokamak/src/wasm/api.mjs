import * as host from "./host";
import { markNonTransferableBuffer, isNonTransferableBuffer } from "../globals/structured-clone.mjs";

const runtime = host.runtime();
const objects = new Map();
const functions = new WeakMap();
const tables = new WeakMap();
const references = [];
const referenceIds = new Map();
const negativeZero = Symbol();
const NULL = 0xffffffff;
const modules = new WeakMap();
const instances = new WeakMap();
const memories = new WeakMap();
const globals = new WeakMap();

for (const name of ["transfer", "transferToFixedLength", "transferToImmutable"]) {
  const original = ArrayBuffer.prototype[name];
  if (!original) continue;
  Object.defineProperty(ArrayBuffer.prototype, name, { configurable: true, writable: true,
    value: function (...args) {
      if (isNonTransferableBuffer(this)) throw new TypeError("WebAssembly memory cannot be transferred");
      return Reflect.apply(original, this, args);
    },
  });
}

class CompileError extends Error { constructor(message) { super(message); this.name = "CompileError"; } }
class LinkError extends Error { constructor(message) { super(message); this.name = "LinkError"; } }
class RuntimeError extends Error { constructor(message) { super(message); this.name = "RuntimeError"; } }

function requireState(map, value) {
  const state = map.get(value);
  if (!state) throw new TypeError("Incompatible WebAssembly receiver");
  return state;
}
function bytes(value) {
  if (value instanceof ArrayBuffer) return value;
  if (ArrayBuffer.isView(value)) return value.buffer.slice(value.byteOffset, value.byteOffset + value.byteLength);
  throw new TypeError("Expected a BufferSource");
}
function disabled() { throw new CompileError("Wasm code generation disallowed by embedder; import a .wasm module"); }
function index(value) {
  const number = +value;
  const result = Number.isNaN(number) ? 0 : Math.trunc(number);
  if (result < 0 || result > 0xffffffff) throw new RangeError("WebAssembly index is out of range");
  return result;
}
function convert(kind, value) {
  switch (kind) {
    case 0: return value | 0;
    case 1: return BigInt.asIntN(64, value);
    case 2: return Math.fround(+value);
    case 3: return +value;
    case 128: {
      if (value === null) return NULL;
      const key = Object.is(value, -0) ? negativeZero : value;
      if (!referenceIds.has(key)) { referenceIds.set(key, references.length); references.push(value); }
      return referenceIds.get(key);
    }
    case 129:
      if (value === null) return NULL;
      if (!functions.has(value)) throw new TypeError("Expected a WebAssembly function or null");
      return functions.get(value);
    default: throw new TypeError("Unsupported WebAssembly value type");
  }
}
function decode(kind, value) {
  if (kind !== 128 && kind !== 129) return value;
  if (value === NULL) return null;
  return kind === 128 ? references[value] : objectFor(value);
}
function objectFor(id) {
  if (objects.has(id)) return objects.get(id);
  const type = host.objectInfo(runtime, id);
  let value;
  if (type.kind === "function") {
    value = (...args) => {
      if (type.results.includes(4)) throw new TypeError("SIMD values cannot cross the JavaScript boundary");
      const result = host.call(runtime, id, type.params.map((kind, i) => convert(kind, args[i])));
      if (type.results.length === 1) return decode(type.results[0], result);
      if (type.results.length > 1) return result.map((value, i) => decode(type.results[i], value));
    };
    Object.defineProperties(value, { length: { value: type.params.length }, name: { value: String(type.function_index) } });
    functions.set(value, id);
  } else {
    const [Constructor, states] = { memory: [Memory, memories], global: [Global, globals], table: [Table, tables] }[type.kind];
    value = Object.create(Constructor.prototype);
    states.set(value, { id, kind: type.value, mutable: type.mutable });
  }
  objects.set(id, value);
  return value;
}
function metadata(module, imports) {
  return host.info(requireState(modules, module), imports);
}

class Module {
  constructor(value) { bytes(value); disabled(); }
  static imports(module) { return metadata(module, true).map(({ module, name, kind }) => ({ module, name, kind })); }
  static exports(module) { return metadata(module, false).map(({ name, kind }) => ({ name, kind })); }
  static customSections(module, sectionName) {
    requireState(modules, module);
    return customSections(moduleBytes.get(module), String(sectionName));
  }
}
const moduleBytes = new WeakMap();
export function compiledModule(buffer) {
  const module = Object.create(Module.prototype);
  modules.set(module, host.load(runtime, buffer));
  moduleBytes.set(module, buffer);
  return module;
}

function resolveImports(module, imports) {
  return metadata(module, true).map(type => {
    if (imports === undefined) throw new TypeError("WebAssembly imports are required");
    const namespace = imports[type.module];
    if ((typeof namespace !== "object" || namespace === null) && typeof namespace !== "function")
      throw new TypeError(`WebAssembly import module ${type.module} is not an object`);
    const value = namespace[type.name];
    if (type.kind !== "function") {
      const states = { global: globals, memory: memories, table: tables }[type.kind];
      if (states.has(value)) return states.get(value).id;
      if (type.kind === "global" && !type.mutable) {
        if (type.value === 128 || (type.value === 129 && (value === null || functions.has(value))) || typeof value === (type.value === 1 ? "bigint" : "number"))
          return host.globalNew(runtime, type.value, false, convert(type.value, value));
      }
      throw new LinkError(`WebAssembly import ${type.module}.${type.name} must be a matching ${type.kind}`);
    }
    if (typeof value !== "function") throw new LinkError(`WebAssembly import ${type.module}.${type.name} is not callable`);
    if (functions.has(value)) return functions.get(value);
    return (...args) => {
      if (type.params.includes(4) || type.results.includes(4)) throw new TypeError("SIMD values cannot cross the JavaScript boundary");
      const result = value(...args.map((value, i) => decode(type.params[i], value)));
      if (type.results.length === 0) return undefined;
      if (type.results.length === 1) return convert(type.results[0], result);
      const values = [...result];
      if (values.length !== type.results.length) throw new TypeError("Wrong WebAssembly import result count");
      return values.map((value, i) => convert(type.results[i], value));
    };
  });
}
class Instance {
  constructor(module, imports) {
    const source = requireState(modules, module);
    if (imports !== undefined && (imports === null || !["object", "function"].includes(typeof imports)))
      throw new TypeError("WebAssembly imports must be an object");
    const handle = host.instantiate(runtime, source, resolveImports(module, imports));
    const exports = Object.create(null);
    for (const [index, type] of metadata(module, false).entries())
      Object.defineProperty(exports, type.name, { value: objectFor(host.exportId(handle, index)), enumerable: true });
    instances.set(this, Object.freeze(exports));
  }
  get exports() { return requireState(instances, this); }
}

function leb(value) {
  const output = [];
  do { const byte = value & 127; value = Math.floor(value / 128); output.push(byte | (value ? 128 : 0)); } while (value);
  return output;
}
class Memory {
  constructor(descriptor) {
    if (descriptor === null || (typeof descriptor !== "object" && typeof descriptor !== "function")) throw new TypeError("Expected a memory descriptor");
    if (descriptor.address !== undefined && descriptor.address !== "i32") throw new TypeError("Only 32-bit WebAssembly memory is supported");
    const shared = Boolean(descriptor.shared);
    if (shared && descriptor.maximum === undefined) throw new TypeError("Shared memory requires a maximum");
    if (descriptor.initial === undefined) throw new TypeError("Memory initial size is required");
    const initial = index(descriptor.initial);
    const maximum = descriptor.maximum === undefined ? undefined : index(descriptor.maximum);
    if (initial > 65536 || (maximum !== undefined && (maximum > 65536 || maximum < initial))) throw new RangeError("Invalid memory limits");
    const limits = maximum === undefined ? [0, ...leb(initial)] : [shared ? 3 : 1, ...leb(initial), ...leb(maximum)];
    const binary = new Uint8Array([0,97,115,109,1,0,0,0,5,...leb(1 + limits.length),1,...limits,7,10,1,6,109,101,109,111,114,121,2,0]);
    const memory = new Instance(compiledModule(binary.buffer)).exports.memory;
    const state = memories.get(memory);
    memories.set(this, state);
    objects.set(state.id, this);
  }
  get buffer() {
    const { id } = requireState(memories, this);
    const buffer = host.memory(runtime, id, undefined);
    markNonTransferableBuffer(buffer);
    return buffer;
  }
  toResizableBuffer() { return memoryBuffer(this, true); }
  toFixedLengthBuffer() { return memoryBuffer(this, false); }
  grow(delta) { if (!arguments.length) throw new TypeError("Memory.grow requires a size"); const state = requireState(memories, this); return host.grow(runtime, state.id, index(delta)); }
}
function memoryBuffer(memory, resizable) {
  const buffer = host.memory(runtime, requireState(memories, memory).id, resizable);
  markNonTransferableBuffer(buffer);
  return buffer;
}
for (const [prototype, method, resizable] of [[ArrayBuffer.prototype, "resize", "resizable"], [SharedArrayBuffer.prototype, "grow", "growable"]]) {
  const original = prototype[method];
  const length = Object.getOwnPropertyDescriptor(prototype, "byteLength").get;
  const maximum = Object.getOwnPropertyDescriptor(prototype, "maxByteLength").get;
  const enabled = Object.getOwnPropertyDescriptor(prototype, resizable).get;
  Object.defineProperty(prototype, method, { configurable: true, writable: true, value: function (size) {
    const id = host.bufferMemory(runtime, this);
    if (id === undefined) return Reflect.apply(original, this, [size]);
    // The native getter checks the receiver before argument conversion.
    Reflect.apply(length, this, []);
    const requested = index(size);
    if (host.bufferMemory(runtime, this) !== id) throw new TypeError("Detached memory buffer");
    if (!Reflect.apply(enabled, this, [])) throw new TypeError("Memory buffer is not resizable");
    const current = Reflect.apply(length, this, []);
    if (requested < current || requested % 65536 || requested > Reflect.apply(maximum, this, [])) throw new RangeError("Invalid memory buffer length");
    host.grow(runtime, id, (requested - current) / 65536);
  }});
}
function valueKind(name) {
  const kind = { i32: 0, i64: 1, f32: 2, f64: 3, externref: 128, anyfunc: 129 }[String(name)];
  if (kind === undefined) throw new TypeError("Unsupported WebAssembly value type");
  return kind;
}
class Global {
  constructor(descriptor, value) {
    if (descriptor === null || (typeof descriptor !== "object" && typeof descriptor !== "function")) throw new TypeError("Expected a global descriptor");
    const kind = valueKind(descriptor.value), mutable = Boolean(descriptor.mutable);
    if (arguments.length < 2 || (value === undefined && kind < 128)) value = kind === 1 ? 0n : kind === 128 ? undefined : kind === 129 ? null : 0;
    const id = host.globalNew(runtime, kind, mutable, convert(kind, value));
    globals.set(this, { id, kind, mutable }); objects.set(id, this);
  }
  get value() { const { id, kind } = requireState(globals, this); return decode(kind, host.globalGet(runtime, id)); }
  set value(value) {
    const state = requireState(globals, this);
    if (!state.mutable) throw new TypeError("WebAssembly global is immutable");
    host.globalSet(runtime, state.id, convert(state.kind, value));
  }
  valueOf() { const { id, kind } = requireState(globals, this); return decode(kind, host.globalGet(runtime, id)); }
}
class Table {
  constructor(descriptor, value) {
    if (descriptor === null || (typeof descriptor !== "object" && typeof descriptor !== "function")) throw new TypeError("Expected a table descriptor");
    if (descriptor.address !== undefined && descriptor.address !== "i32") throw new TypeError("Only 32-bit WebAssembly tables are supported");
    const kind = valueKind(descriptor.element);
    if (kind !== 128 && kind !== 129) throw new TypeError("Invalid WebAssembly table element type");
    if (descriptor.initial === undefined) throw new TypeError("Table initial size is required");
    const initial = index(descriptor.initial), maximum = descriptor.maximum === undefined ? NULL : index(descriptor.maximum);
    if (maximum < initial) throw new RangeError("Invalid table limits");
    if (value === undefined) value = kind === 128 ? undefined : null;
    const id = host.tableNew(runtime, kind, initial, maximum, convert(kind, value));
    tables.set(this, { id, kind }); objects.set(id, this);
  }
  get length() { return host.tableSize(runtime, requireState(tables, this).id); }
  get(position) { if (!arguments.length) throw new TypeError("Table.get requires an index"); const state = requireState(tables, this); return decode(state.kind, host.tableGet(runtime, state.id, index(position))); }
  set(position, value) {
    if (!arguments.length) throw new TypeError("Table.set requires an index");
    const state = requireState(tables, this);
    if (arguments.length < 2) value = state.kind === 128 ? undefined : null;
    host.tableSet(runtime, state.id, index(position), convert(state.kind, value));
  }
  grow(delta, value) {
    if (!arguments.length) throw new TypeError("Table.grow requires a size");
    const state = requireState(tables, this);
    if (arguments.length < 2) value = state.kind === 128 ? undefined : null;
    return host.tableGrow(runtime, state.id, index(delta), convert(state.kind, value));
  }
}
function customSections(buffer, name) {
  const data = new Uint8Array(buffer);
  const output = [];
  let cursor = 8;
  function uint() {
    let value = 0, shift = 0, byte;
    do { byte = data[cursor++]; value += (byte & 127) * 2 ** shift; shift += 7; } while (byte & 128);
    return value;
  }
  while (cursor < data.length) {
    const kind = data[cursor++], length = uint(), end = cursor + length;
    if (kind === 0) {
      const length = uint(), start = cursor;
      cursor += length;
      if (new TextDecoder().decode(data.subarray(start, cursor)) === name) output.push(buffer.slice(cursor, end));
    }
    cursor = end;
  }
  return output;
}
for (const constructor of [Module, Instance, Memory, Global, Table])
  Object.defineProperty(constructor.prototype, Symbol.toStringTag, { value: `WebAssembly.${constructor.name}`, configurable: true });
globalThis.WebAssembly = {
  Module, Instance, Memory, Global, Table, CompileError, LinkError, RuntimeError,
  async instantiate(module, imports) { if (!modules.has(module)) { bytes(module); disabled(); } return new Instance(module, imports); },
  async compile(value) { bytes(value); disabled(); },
  validate(value) { const buffer = bytes(value); try { host.load(host.runtime(), buffer); return true; } catch (error) { if (error instanceof CompileError) return false; throw error; } },
  [Symbol.toStringTag]: "WebAssembly",
};
