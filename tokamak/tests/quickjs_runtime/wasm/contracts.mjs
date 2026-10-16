import module from "./core.wasm";

export default { async fetch() {
  const calls = [];
  const sentinel = { sentinel: true };
  const instance = await WebAssembly.instantiate(module, { env: {
    notify(value) { calls.push(value); if (value === -1) throw sentinel; return value * 2; },
    limit: 11,
  } });
  const e = instance.exports;
  const out = {
    module: module instanceof WebAssembly.Module,
    instance: instance instanceof WebAssembly.Instance,
    exports: WebAssembly.Module.exports(module),
    imports: WebAssembly.Module.imports(module),
    add: e.add(20, 22), call: e.callback(7), calls,
    wide: String(e.wide(9007199254740993n)), float: e.float(1.25, 2.5),
    pair: e.pair().map(String), limit: e.limit(), counter: e.increment(),
    frozen: Object.isFrozen(e), nullPrototype: Object.getPrototypeOf(e) === null,
  };
  e.counter.value = 20;
  out.counterSet = e.increment();
  const view = new Uint32Array(e.memory.buffer);
  view[0] = 123;
  out.memoryRead = e.load(0);
  e.store(4, 456);
  out.memoryWrite = view[1];
  out.sameBuffer = e.memory.buffer === view.buffer;
  out.grow = e.grow(1);
  out.detached = view.buffer.byteLength;
  out.pages = e.memory.buffer.byteLength / 65536;
  out.retained = new Uint32Array(e.memory.buffer)[0];
  const before = e.memory.buffer;
  out.growZero = e.memory.grow(0);
  out.growZeroDetached = before.byteLength;
  const zeroBefore = e.memory.buffer;
  out.wasmGrowZero = e.grow(0);
  out.wasmGrowZeroDetached = zeroBefore.byteLength;
  const aliasBefore = e.alias.buffer;
  out.aliasGrow = e.memory.grow(0);
  out.aliasDetached = aliasBefore.byteLength;
  out.errors = {};
  for (const [name, action] of Object.entries({
    trap: () => e.trap(), bounds: () => e.load(200000),
    grow: () => e.memory.grow(9), wrongBigInt: () => e.wide(3),
    missing: () => new WebAssembly.Instance(module, {}),
    nonFunction: () => new WebAssembly.Instance(module, { env: { notify: 3, limit: 1 } }),
    compile: () => new WebAssembly.Module(new Uint8Array([0,97,115,109,1,0,0,0])),
    transfer: () => ArrayBuffer.prototype.transfer.call(e.memory.buffer),
    cloneTransfer: () => structuredClone(e.memory.buffer, { transfer: [e.memory.buffer] }),
  })) { try { action(); out.errors[name] = "none"; } catch (error) { out.errors[name] = error.name; } }
  try { e.callback(-1); } catch (error) { out.exceptionIdentity = error === sentinel; }
  out.afterTrap = e.add(1, 2);
  const other = new WebAssembly.Instance(module, { env: { notify: x => x, limit: 2 } });
  out.isolatedMemory = new Uint32Array(other.exports.memory.buffer)[0];
  let callbackInstance, callbackBuffer;
  callbackInstance = new WebAssembly.Instance(module, { env: {
    notify(value) {
      if (value !== 99) return value;
      out.callbackDetached = callbackBuffer.byteLength;
      out.callbackPages = callbackInstance.exports.memory.buffer.byteLength / 65536;
      return callbackInstance.exports.add(10, 20);
    }, limit: 0,
  } });
  callbackBuffer = callbackInstance.exports.memory.buffer;
  out.reentrant = callbackInstance.exports.growAndCallback(1);
  const zeroMemory = new WebAssembly.Memory({ initial: 0, maximum: 1 });
  const emptyBuffer = zeroMemory.buffer;
  out.sameEmptyBuffer = emptyBuffer === zeroMemory.buffer;
  out.emptyMemory = [emptyBuffer.byteLength, zeroMemory.grow(1), zeroMemory.buffer.byteLength];
  const memory = new WebAssembly.Memory({ initial: 1, maximum: 2 });
  new Uint8Array(memory.buffer)[0] = 31;
  out.newMemory = [memory.grow(1), memory.buffer.byteLength, new Uint8Array(memory.buffer)[0]];
  const global = new WebAssembly.Global({ value: "i64", mutable: true }, 4n);
  global.value = 7n;
  out.newGlobal = String(global.value);
  out.validate = [WebAssembly.validate(new Uint8Array([0,97,115,109,1,0,0,0])), WebAssembly.validate(new Uint8Array([1,2,3]))];
  return Response.json(out);
} };
