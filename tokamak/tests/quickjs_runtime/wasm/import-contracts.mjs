import module from './imports.wasm';
import simd from './simd.wasm';
import forward from './forward.wasm';
import failed from './failed.wasm';
function check(condition, label) { if (!condition) throw new Error(label); }
function throws(fn, kind) { try { fn(); } catch (e) { check(e.name === kind, `${kind}: ${e}`); return; } throw new Error(`missing ${kind}`); }
export default { fetch() { try {
  check(WebAssembly.Module.imports(module).map(i => i.name).join(',') === 'memory,table,refs,counter,reference,notify,pair', 'import declaration order');
  const memory = new WebAssembly.Memory({ initial: 1, maximum: 4 });
  const table = new WebAssembly.Table({ initial: 1, maximum: 8, element: 'anyfunc' });
  const refs = new WebAssembly.Table({ initial: 1, maximum: 8, element: 'externref' });
  const counter = new WebAssembly.Global({ value: 'i32', mutable: true }, 10);
  const object = { hello: 'wasm' };
  const reference = new WebAssembly.Global({ value: 'externref', mutable: true }, object);
  let oldBuffer = memory.buffer, calls = 0;
  const env = { memory, table, refs, counter, reference, pair: () => [7, 9n, 2.5], notify() {
    calls++;
    check(new Uint32Array(memory.buffer)[0] === 42, 'start sees imported memory');
    if (calls > 2) check(oldBuffer.byteLength === 0, 'callback sees detached buffer');
    if (calls === 2) { check(counter.value === 11, 'callback sees Wasm mutation'); counter.value = 20; }
  }};
  const a = new WebAssembly.Instance(module, { env }).exports;
  check(a.memory === memory && a.table === table && a.refs === refs && a.counter === counter && a.reference === reference, 'import export identity');
  check(a.double === a.alias && table.get(0) === a.double && a.functionRef() === a.double, 'function identity');
  check(a.bump() === 20 && counter.value === 20, 'shared mutable global');
  check(a.indirect(0, 21) === 42, 'initialized imported table');
  check(a.pair().join(',') === '7,9,2.5', 'multi-result import');
  check(a.echoRef(object) === object, 'reference function');
  for (const value of [undefined, null, false, 0, -0, NaN, 17, 42n, 'hello', Symbol('ref'), object]) {
    a.refSet(value);
    check(Object.is(a.refGet(), value) && Object.is(reference.value, value) && Object.is(refs.get(0), value), `reference identity ${String(value)}: ${String(a.refGet())}, ${String(reference.value)}, ${String(refs.get(0))}`);
  }
  check(table.grow(1, a.double) === 1 && table.length === 2, 'host table growth');
  check(a.tableGrow(1) === 2 && table.get(2) === a.double, 'Wasm table growth');
  let secondaryNotify = () => {};
  const secondEnv = { ...env, notify() { secondaryNotify(); }, counter: new WebAssembly.Global({ value: 'i32', mutable: true }, 100) };
  const b = new WebAssembly.Instance(module, { env: secondEnv }).exports;
  check(table.get(0) === b.double && a.indirect(0, 21) === 42, 'cross-instance table initialization');
  a.tableSet(1, b.double);
  check(a.tableGet(1) === b.double && a.indirect(1, 21) === 42, 'cross-instance function reference');
  throws(() => b.trap(), "RuntimeError");
  table.set(0, b.withGlobal); check(a.indirect(0, 1) === 101, 'function uses its owning instance globals');
  table.set(0, null); throws(() => a.indirect(0, 1), 'RuntimeError');
  a.init(); check(table.get(0) === a.double, 'passive element initialization');
  throws(() => table.set(0, () => 1), 'TypeError');
  throws(() => table.get(100), 'RangeError');
  throws(() => table.grow(10), 'RangeError');
  throws(() => new WebAssembly.Instance(module, { env: { ...env, memory: new WebAssembly.Memory({ initial: 0, maximum: 4 }) }}), 'LinkError');
  throws(() => new WebAssembly.Instance(module, { env: { ...env, counter: 1 }}), 'LinkError');
  for (const override of [
    { memory: new WebAssembly.Memory({ initial: 1, maximum: 6 }) },
    { memory: new WebAssembly.Memory({ initial: 1 }) },
    { table: new WebAssembly.Table({ element: 'externref', initial: 1, maximum: 8 }) },
    { table: new WebAssembly.Table({ element: 'anyfunc', initial: 0, maximum: 8 }) },
    { table: new WebAssembly.Table({ element: 'anyfunc', initial: 1, maximum: 11 }) },
    { table: new WebAssembly.Table({ element: 'anyfunc', initial: 1 }) },
    { counter: new WebAssembly.Global({ value: 'f32', mutable: true }, 1) },
    { counter: new WebAssembly.Global({ value: 'i32' }, 1) },
    { reference: new WebAssembly.Global({ value: 'externref' }, object) },
    { notify: b.double },
    { pair: b.double },
  ]) throws(() => new WebAssembly.Instance(module, { env: { ...env, ...override } }), 'LinkError');
  check(a.grow(1) === 1 && memory.buffer.byteLength === 131072 && b.memory.buffer === memory.buffer, 'shared imported memory growth');
  oldBuffer = memory.buffer;
  check(b.memory.grow(0) === 2 && oldBuffer.byteLength === 0, 'growth through another export');
  const wide = new WebAssembly.Global({ value: 'i64', mutable: true }, 9007199254740993n);
  const f = new WebAssembly.Instance(forward, { other: {
    function: b.withGlobal, reference: object, wide,
    refCallback(value, func) { check(value === object && func === a.double, 'callback receives references'); return [value, func]; }
  }}).exports;
  check(f.alias === b.withGlobal && f.functionGlobal.value === b.withGlobal, 'function import identity');
  check(f.call(2) === 102 && f.tail(3) === 103 && f.indirect(4) === 104, 'cross-instance direct and tail calls');
  check(f.reference.value === object && f.nullGlobal.value === null, 'defined reference globals');
  f.nullGlobal.value = undefined; check(f.nullGlobal.value === undefined, 'undefined reference global');
  check(f.table.get(0) === b.withGlobal && f.table.grow(1, a.double) === 1, 'exported table host growth');
  check(f.wide() === 9007199254740994n && wide.value === 9007199254740994n, 'mutable i64 import');
  const pair = f.refs(object, a.double); check(pair[0] === object && pair[1] === a.double, 'multi-result reference callback');
  check(f.refsTable.get(0) === object, 'active element global initializer');
  f.refsTable.set(0, null); f.initRefs(); check(f.refsTable.get(0) === object, 'passive element global initializer');
  const escaped = new WebAssembly.Table({ element: 'anyfunc', initial: 1 });
  throws(() => new WebAssembly.Instance(failed, { env: { table: escaped, value: () => 42 } }), 'RuntimeError');
  check(escaped.get(0)() === 42, 'functions escaping failed instantiation remain callable');
  table.set(0, b.grow);
  secondaryNotify = () => { check(a.double(21) === 42, 'foreign callback reentry'); };
  const prior = memory.buffer;
  check(a.indirect(0, 0) === 2 && prior.byteLength === 0, 'foreign callback sees memory growth');
  const sentinel = { thrown: true };
  secondaryNotify = () => { check(a.double(21) === 42, 'reentry before exception'); throw sentinel; };
  let caught;
  try { a.indirect(0, 0); } catch (error) { caught = error; }
  check(caught === sentinel, 'foreign callback exception identity');
  secondaryNotify = () => {};
  table.set(0, b.withGlobal); check(a.indirect(0, 1) === 101, 'foreign call recovers after callback exception');
  const s = new WebAssembly.Instance(simd).exports;
  check(s.tail() === 42, 'SIMD tail call');
  throws(() => s.vector(), 'TypeError'); check(s.calls.value === 0, 'SIMD JS result rejects before executing');
  check(s.sum(38) === 42, 'SIMD arithmetic'); s.store();
  check([...new Uint32Array(s.memory.buffer, 0, 4)].join(',') === '10,20,30,40', 'SIMD memory');
  return Response.json({ imports: true, tables: true, globals: true, references: true, simd: true });
} catch (error) { return Response.json({ error: error.name, message: error.message, stack: error.stack }); } }};
