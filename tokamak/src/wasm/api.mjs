import * as host from "./host";
import { markNonTransferableBuffer, isNonTransferableBuffer } from "../globals/structured-clone.mjs";

const modules = new WeakMap();
const instances = new WeakMap();
const memories = new WeakMap();
const globals = new WeakMap();
const internal = Symbol();

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
    default: throw new TypeError("Unsupported WebAssembly value type");
  }
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
  modules.set(module, host.load(buffer));
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
    if (type.kind === "global") {
      if (globals.has(value)) {
        const state = globals.get(value);
        if (state.kind !== type.value || state.mutable !== type.mutable) throw new LinkError("WebAssembly global import type mismatch");
        return value.value;
      }
      if (typeof value !== (type.value === 1 ? "bigint" : "number")) throw new LinkError("WebAssembly global import must be a number or matching Global");
      return value;
    }
    if (type.kind !== "function") return value;
    if (typeof value !== "function") throw new LinkError(`WebAssembly import ${type.module}.${type.name} is not callable`);
    return (...args) => {
      const result = value(...args);
      if (type.results.length === 0) return undefined;
      if (type.results.length === 1) return convert(type.results[0], result);
      const values = Array.from(result);
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
    const handle = host.instantiate(source, resolveImports(module, imports));
    const exports = Object.create(null);
    for (const [index, type] of metadata(module, false).entries()) {
      let value;
      switch (type.kind) {
        case "function":
          value = (...args) => host.call(handle, index, type.params.map((kind, i) => convert(kind, args[i])));
          Object.defineProperties(value, { length: { value: type.params.length }, name: { value: type.name } });
          break;
        case "memory": value = new Memory(internal, { handle, index }); break;
        case "global": value = new Global(internal, { handle, index, kind: type.value, mutable: type.mutable }); break;
        case "table": value = new Table(internal, { handle, index }); break;
      }
      Object.defineProperty(exports, type.name, { value, enumerable: true });
    }
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
  constructor(descriptor, state) {
    if (descriptor === internal) { memories.set(this, state); return; }
    if (descriptor === null || typeof descriptor !== "object") throw new TypeError("Expected a memory descriptor");
    if (descriptor.shared) throw new TypeError("Shared WebAssembly memory is not supported");
    if (descriptor.initial === undefined) throw new TypeError("Memory initial size is required");
    const initial = index(descriptor.initial);
    const maximum = descriptor.maximum === undefined ? undefined : index(descriptor.maximum);
    if (initial > 65536 || (maximum !== undefined && (maximum > 65536 || maximum < initial))) throw new RangeError("Invalid memory limits");
    const limits = maximum === undefined ? [0, ...leb(initial)] : [1, ...leb(initial), ...leb(maximum)];
    const binary = new Uint8Array([0,97,115,109,1,0,0,0,5,...leb(1 + limits.length),1,...limits,7,10,1,6,109,101,109,111,114,121,2,0]);
    const memory = new Instance(compiledModule(binary.buffer)).exports.memory;
    memories.set(this, memories.get(memory));
  }
  get buffer() {
    const { handle, index } = requireState(memories, this);
    const buffer = host.memory(handle, index);
    markNonTransferableBuffer(buffer);
    return buffer;
  }
  grow(delta) { const state = requireState(memories, this); return host.grow(state.handle, state.index, index(delta)); }
}
class Global {
  constructor(descriptor, value) {
    if (descriptor === internal) { globals.set(this, value); return; }
    if (descriptor === null || typeof descriptor !== "object") throw new TypeError("Expected a global descriptor");
    const kind = ["i32", "i64", "f32", "f64"].indexOf(String(descriptor.value));
    if (kind < 0) throw new TypeError("Unsupported WebAssembly global type");
    globals.set(this, { kind, mutable: Boolean(descriptor.mutable), value: convert(kind, value === undefined ? (kind === 1 ? 0n : 0) : value) });
  }
  get value() { const state = requireState(globals, this); return state.handle ? host.globalGet(state.handle, state.index) : state.value; }
  set value(value) {
    const state = requireState(globals, this);
    if (state.handle) { host.globalSet(state.handle, state.index, convert(state.kind, value)); return; }
    if (!state.mutable) throw new TypeError("WebAssembly global is immutable");
    state.value = convert(state.kind, value);
  }
  valueOf() { return this.value; }
}
class Table {
  constructor(token) { if (token !== internal) throw new TypeError("WebAssembly.Table construction is not supported by this WAMR spike"); }
  get length() { throw new TypeError("Host access to WebAssembly tables is not supported by this WAMR spike"); }
  get() { throw new TypeError("Host access to WebAssembly tables is not supported by this WAMR spike"); }
  set() { throw new TypeError("Host access to WebAssembly tables is not supported by this WAMR spike"); }
  grow() { throw new TypeError("Host access to WebAssembly tables is not supported by this WAMR spike"); }
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
  validate(value) { const buffer = bytes(value); try { host.load(buffer); return true; } catch (error) { if (error instanceof CompileError) return false; throw error; } },
  [Symbol.toStringTag]: "WebAssembly",
};
