import sharedModule from './shared.wasm';
function check(condition, label) { if (!condition) throw new Error(label); }
function throws(fn, name) { try { fn(); } catch(e) { check(e.name === name, `${name}: ${e}`); return; } throw new Error(`missing ${name}`); }
export default { fetch() { try {
  for (const shared of [false, true]) {
    const memory = new WebAssembly.Memory({ initial: 1, maximum: 4, shared });
    const fixed = memory.buffer, fixedView = new Uint8Array(fixed);
    fixedView[0] = 42;
    const buffer = memory.toResizableBuffer(), view = new Uint8Array(buffer), limited = new Uint8Array(buffer, 0, 4), data = new DataView(buffer);
    check(buffer === memory.buffer && buffer === memory.toResizableBuffer(), 'resizable identity');
    check(buffer.maxByteLength === 262144 && buffer[shared ? 'growable' : 'resizable'], 'resizable maximum');
    check(fixed.byteLength === (shared ? 65536 : 0), 'conversion detachment');
    check(view[0] === 42, 'conversion keeps data');
    const resize = n => shared ? buffer.grow(n) : buffer.resize(n);
    throws(() => resize(65537), 'RangeError');
    resize(131072);
    check(memory.buffer === buffer && view.length === 131072 && data.byteLength === 131072 && limited.length === 4, 'tracking views');
    view[65536] = 91;
    check(new Uint8Array(memory.buffer)[65536] === 91, 'grown contents');
    throws(() => resize(65536), 'RangeError');
    throws(() => resize(327680), 'RangeError');
    check(memory.grow(0) === 2 && memory.buffer === buffer, 'resizable zero growth');
    const clone = shared ? structuredClone(buffer) : null;
    const next = memory.toFixedLengthBuffer();
    check(next === memory.buffer && next === memory.toFixedLengthBuffer(), 'fixed identity');
    check(buffer.byteLength === (shared ? 131072 : 0), 'fixed conversion detachment');
    const again = memory.toResizableBuffer();
    check(again !== buffer, 'mode conversion creates new wrapper');
    check(memory.grow(1) === 2 && again.byteLength === 196608, 'resizable memory growth');
    if (shared) {
      check(buffer.byteLength === 196608 && clone.byteLength === 196608, 'old shared growable wrappers track memory');
      clone.grow(262144);
      check(again.byteLength === 262144 && view.length === 262144, 'cloned shared growth');
      fixedView[0] = 67; check(new Uint8Array(again)[0] === 67, 'old fixed shared view remains alive');
    }
    throws(() => ArrayBuffer.prototype.resize.call(shared ? again : {}, 0), 'TypeError');
  }
  const empty = new WebAssembly.Memory({ initial: 0, maximum: 1 });
  const emptyBuffer = empty.toResizableBuffer(), emptyView = new Uint8Array(emptyBuffer);
  emptyBuffer.resize(65536); check(emptyView.length === 65536, 'empty memory growth');
  const zeroShared = new WebAssembly.Memory({initial:0, maximum:0, shared:true});
  check(zeroShared.toResizableBuffer().byteLength === 0, 'zero shared maximum');
  const reentrant = new WebAssembly.Memory({initial:0, maximum:1});
  const abandoned = reentrant.toResizableBuffer();
  throws(() => abandoned.resize({valueOf() { reentrant.toFixedLengthBuffer(); return 65536; }}), 'TypeError');
  check(reentrant.buffer.byteLength === 0, 'detached during size conversion');
  const unlimited = new WebAssembly.Memory({ initial: 0 });
  throws(() => unlimited.toResizableBuffer(), 'TypeError');
  const largeMaximum = new WebAssembly.Memory({initial:0, maximum:65536}).toResizableBuffer();
  check(largeMaximum.maxByteLength === 4294967296 && structuredClone(largeMaximum).maxByteLength === 4294967296, 'large maximum clone');
  const memory = new WebAssembly.Memory({ initial: 1, maximum: 4, shared: true });
  const a = new WebAssembly.Instance(sharedModule, { env: { memory } }).exports;
  const b = new WebAssembly.Instance(sharedModule, { env: { memory } }).exports;
  check(a.memory === memory && b.memory === memory && memory.buffer instanceof SharedArrayBuffer, 'shared imports');
  const old = memory.buffer, ints = new Int32Array(old);
  Atomics.store(ints, 0, 41); check(a.add(0, 1) === 41 && Atomics.load(ints, 0) === 42, 'atomic add');
  check(b.exchange(0, 42, 99) === 42 && a.load(0) === 99, 'atomic compare exchange');
  a.store(4, 123); check(Atomics.load(ints, 1) === 123, 'atomic store');
  check(a.add64(8, 9007199254740993n) === 0n && new BigInt64Array(old)[1] === 9007199254740993n, 'atomic i64');
  throws(() => a.load(1), 'RuntimeError'); throws(() => a.load(65536), 'RuntimeError');
  throws(() => a.wait(0, 99, 0n), 'RuntimeError'); throws(() => a.wait(0, 100, -1n), 'RuntimeError');
  check(a.notify(0, 1) === 0, 'atomic notify'); a.fence();
  memory.grow(0); check(memory.buffer === old, 'fixed shared zero growth');
  check(a.grow(1) === 1 && old.byteLength === 65536 && memory.buffer.byteLength === 131072, 'fixed shared growth');
  const growable = memory.toResizableBuffer(), tracking = new Uint8Array(growable);
  check(b.grow(1) === 2 && tracking.length === 196608, 'Wasm growth updates tracking views');
  check(a.grow(2) === -1 && tracking.length === 196608, 'failed growth preserves buffer');
  throws(() => new WebAssembly.Memory({ initial: 1, shared: true }), 'TypeError');
  throws(() => new WebAssembly.Instance(sharedModule, { env: { memory: new WebAssembly.Memory({ initial: 1, maximum: 4 }) } }), 'LinkError');
  return Response.json({ sharedMemory: true, atomics: true, resizableMemory: true });
} catch (e) { return Response.json({ error: e.name, message: e.message, stack: e.stack }); } }};
