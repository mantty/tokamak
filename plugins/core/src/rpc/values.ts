/** A typed array or `DataView` constructor, which RPC views name. */
interface ViewConstructor {
  readonly name: string;
  readonly prototype: ArrayBufferView;
  readonly BYTES_PER_ELEMENT?: number;
  new (buffer: ArrayBuffer, byteOffset: number, length: number): ArrayBufferView;
}

const arrays: ViewConstructor[] = [
  Int8Array, Uint8Array, Uint8ClampedArray, Int16Array, Uint16Array,
  Int32Array, Uint32Array, Float32Array, Float64Array, BigInt64Array, BigUint64Array,
];
if (typeof Float16Array !== 'undefined') arrays.push(Float16Array);
const views = new Map(arrays.map(Type => [Type.name, Type]));
views.set('DataView', DataView);

/** A value in an encoded graph: JSON primitives as themselves, other values tagged. */
type Reference = null | string | boolean | number | ['undefined'] | ['bigint', string] | ['number', string] | ['ref', number];

type Properties = [string, Reference][];

/** One object in an encoded graph. */
type Node =
  | ['array', number, Properties]
  | ['record', Properties]
  | ['date', Reference]
  | ['regexp', string, string]
  | ['map', [Reference, Reference][]]
  | ['set', Reference[]]
  | ['error' | 'exception', string, string, Properties]
  | ['buffer' | 'stream', number]
  | ['view', string, Reference, number, number];

/** A value graph as JSON, with the buffers and streams it holds, which the transport carries beside it. */
export interface Message {
  data: string;
  buffers: ArrayBuffer[];
  streams: ReadableStream<unknown>[];
}

export function dataCloneError(): DOMException {
  return new DOMException('Value is outside the supported RPC contract.', 'DataCloneError');
}

/** The property `key` of a peer's object, which must be an object. */
export function field(value: unknown, key: string): unknown {
  if (typeof value !== 'object' || value === null) throw dataCloneError();
  return Reflect.get(value, key);
}

function isList(value: unknown): value is readonly unknown[] {
  return Array.isArray(value);
}

/** A peer's array. */
export function list(value: unknown): readonly unknown[] {
  if (!isList(value)) throw dataCloneError();
  return value;
}

/** Whether a peer's `value` is an index: a non-negative safe integer. */
export function isIndex(value: unknown): value is number {
  return typeof value === 'number' && Number.isSafeInteger(value) && value >= 0;
}

/** A peer's index: a position in a list of `length` items, or any index without one. */
function position(value: unknown, length = Infinity): number {
  if (!isIndex(value) || value >= length) throw dataCloneError();
  return value;
}

function text(value: unknown): string {
  if (typeof value !== 'string') throw dataCloneError();
  return value;
}

function byteStream(value: ReadableStream<unknown>): ReadableStream<unknown> {
  if (value.locked) throw dataCloneError();
  return value;
}

function attachedBuffer(value: ArrayBuffer): ArrayBuffer {
  if (value.detached) throw dataCloneError();
  return value;
}

function viewName(value: ArrayBufferView): string {
  const name = [...views].find(([, Type]) => Object.getPrototypeOf(value) === Type.prototype)?.[0];
  if (!name) throw dataCloneError();
  return name;
}

type Visit = (value: unknown) => Reference;

/** What encoding a graph collects: each value's reference, and the buffers and streams the transport carries. */
interface Encoding {
  visit: Visit;
  buffers: ArrayBuffer[];
  streams: ReadableStream<unknown>[];
}

function properties(value: object, visit: Visit, keys = Object.keys(value)): Properties {
  return keys.map(key => [key, visit(Reflect.get(value, key))]);
}

function errorProperties(value: Error, visit: Visit): Properties {
  const keys = Object.getOwnPropertyNames(value).filter(key => !['name', 'message', 'stack'].includes(key));
  return properties(value, visit, keys);
}

function encodeNumber(value: number): Reference {
  if (Number.isFinite(value) && !Object.is(value, -0)) return value;
  return ['number', Object.is(value, -0) ? '-0' : String(value)];
}

/** A primitive's reference, or undefined for an object or a function. */
function encodePrimitive(value: unknown): Reference | undefined {
  if (value === null || typeof value === 'string' || typeof value === 'boolean') return value;
  if (typeof value === 'undefined') return ['undefined'];
  if (typeof value === 'bigint') return ['bigint', String(value)];
  if (typeof value === 'number') return encodeNumber(value);
  return undefined;
}

/** An array, date, pattern, map or set's node, or undefined for another object. */
function encodeStructure(value: object, visit: Visit): Node | undefined {
  if (Array.isArray(value)) return ['array', value.length, properties(value, visit)];
  if (value instanceof Date) return ['date', visit(Date.prototype.getTime.call(value))];
  if (value instanceof RegExp) return ['regexp', value.source, value.flags];
  if (value instanceof Map) return ['map', [...value].map(([key, entry]): [Reference, Reference] => [visit(key), visit(entry)])];
  if (value instanceof Set) return ['set', [...value].map(visit)];
  return undefined;
}

/** An error, stream or binary value's node, or undefined for another object. */
function encodePlatformObject(value: object, { visit, buffers, streams }: Encoding): Node | undefined {
  if (value instanceof DOMException) return ['exception', value.name, value.message, errorProperties(value, visit)];
  if (value instanceof Error) return ['error', value.name, value.message, errorProperties(value, visit)];
  if (value instanceof ReadableStream) return ['stream', streams.push(byteStream(value)) - 1];
  if (value instanceof ArrayBuffer) return ['buffer', buffers.push(attachedBuffer(value)) - 1];
  if (ArrayBuffer.isView(value)) return ['view', viewName(value), visit(value.buffer), value.byteOffset, value.byteLength];
  return undefined;
}

function encodeRecord(value: object, visit: Visit): Node {
  const prototype: unknown = Object.getPrototypeOf(value);
  if (prototype !== null && prototype !== Object.prototype) throw dataCloneError();
  return ['record', properties(value, visit)];
}

/** Describe a value graph; the transport copies buffers and transfers streams after validation. */
export function encode(value: unknown): Message {
  const nodes: (Node | null)[] = [], seen = new Map<object, number>();
  const encoding: Encoding = { visit, buffers: [], streams: [] };
  function visit(value: unknown): Reference {
    const primitive = encodePrimitive(value);
    if (primitive !== undefined) return primitive;
    if (typeof value !== 'object' || value === null) throw dataCloneError();
    const known = seen.get(value);
    if (known !== undefined) return ['ref', known];
    const index = nodes.length;
    seen.set(value, index);
    nodes.push(null);
    nodes[index] = encodeStructure(value, visit) ?? encodePlatformObject(value, encoding) ?? encodeRecord(value, visit);
    return ['ref', index];
  }
  const root = visit(value);
  return { data: JSON.stringify({ root, nodes }), buffers: encoding.buffers, streams: encoding.streams };
}

type Read = (value: unknown) => unknown;

function setProperties(value: object, entries: unknown, read: Read): void {
  for (const entry of list(entries)) {
    const [key, item] = list(entry);
    Object.defineProperty(value, text(key), { value: read(item), enumerable: true, configurable: true, writable: true });
  }
}

/** A decoded object, and the function that fills it once the graph can refer to it. */
type Allocation = [value: unknown, populate?: () => void];

/** Allocate the object a node describes, given the node's fields after its tag. */
type Allocator = (fields: readonly unknown[], read: Read, message: Message) => Allocation;

const allocators = new Map<unknown, Allocator>([
  ['array', ([length, entries], read) => {
    const array = new Array(position(length));
    return [array, () => { setProperties(array, entries, read); }];
  }],
  ['record', ([entries], read) => {
    const record = {};
    return [record, () => { setProperties(record, entries, read); }];
  }],
  ['map', ([entries], read) => {
    const map = new Map<unknown, unknown>();
    return [map, () => {
      for (const entry of list(entries)) {
        const [key, item] = list(entry);
        map.set(read(key), read(item));
      }
    }];
  }],
  ['set', ([items], read) => {
    const set = new Set<unknown>();
    return [set, () => { for (const item of list(items)) set.add(read(item)); }];
  }],
  ['date', ([encoded], read) => {
    const time = read(encoded);
    if (typeof time !== 'number') throw dataCloneError();
    return [new Date(time)];
  }],
  ['regexp', ([source, flags]) => [new RegExp(text(source), text(flags))]],
  ['error', ([name, message, entries], read) => {
    const error = Object.assign(new Error(text(message)), { name: text(name) });
    return [error, () => { setProperties(error, entries, read); }];
  }],
  ['exception', ([name, message, entries], read) => {
    const exception = new DOMException(text(message), text(name));
    return [exception, () => { setProperties(exception, entries, read); }];
  }],
  ['buffer', ([index], _read, { buffers }) => [buffers[position(index, buffers.length)]]],
  ['stream', ([index], _read, { streams }) => [streams[position(index, streams.length)]]],
  ['view', ([name, encoded, offset, length], read) => {
    const Type = views.get(text(name));
    if (!Type) throw dataCloneError();
    const buffer = read(encoded);
    if (!(buffer instanceof ArrayBuffer)) throw dataCloneError();
    return [new Type(buffer, position(offset), position(length) / (Type.BYTES_PER_ELEMENT ?? 1))];
  }],
]);

function allocate(node: unknown, read: Read, message: Message): Allocation {
  const [tag, ...fields] = list(node);
  const allocator = allocators.get(tag);
  if (!allocator) throw dataCloneError();
  return allocator(fields, read, message);
}

/** Reconstruct a graph using buffers and streams owned by this receiver. */
export function decode(message: Message): unknown {
  const graph: unknown = JSON.parse(message.data);
  const nodes = list(field(graph, 'nodes'));
  const values = new Map<number, unknown>(), pending = new Set<number>();
  function read(value: unknown): unknown {
    if (!isList(value)) return value;
    const [tag, entry] = value;
    if (tag === 'undefined') return undefined;
    if (tag === 'bigint') return BigInt(text(entry));
    if (tag === 'number') return Number(entry);
    if (tag !== 'ref') throw dataCloneError();
    const index = position(entry, nodes.length);
    if (values.has(index)) return values.get(index);
    if (pending.has(index)) throw dataCloneError();
    pending.add(index);
    const [result, populate] = allocate(nodes[index], read, message);
    values.set(index, result);
    populate?.();
    pending.delete(index);
    return result;
  }
  return read(field(graph, 'root'));
}
