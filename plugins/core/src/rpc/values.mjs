const arrays = [
  Int8Array, Uint8Array, Uint8ClampedArray, Int16Array, Uint16Array,
  Int32Array, Uint32Array, Float32Array, Float64Array, BigInt64Array, BigUint64Array,
];
if (typeof Float16Array !== 'undefined') arrays.push(Float16Array);
const views = new Map(arrays.map(Type => [Type.name, Type]));
views.set('DataView', DataView);

export function dataCloneError() {
  return new DOMException('Value is outside the supported RPC contract.', 'DataCloneError');
}

function byteStream(value) {
  if (value.locked) throw dataCloneError();
  return value;
}

function properties(value, visit, keys = Object.keys(value)) {
  return keys.map(key => [key, visit(value[key])]);
}

function errorProperties(value, visit) {
  const keys = Object.getOwnPropertyNames(value).filter(key => !['name', 'message', 'stack'].includes(key));
  return properties(value, visit, keys);
}

function encodeObject(value, visit, buffers, streams) {
  if (Array.isArray(value)) return ['array', value.length, properties(value, visit)];
  if (value instanceof Date) return ['date', visit(Date.prototype.getTime.call(value))];
  if (value instanceof RegExp) return ['regexp', value.source, value.flags];
  if (value instanceof Map) return ['map', [...value].map(([key, entry]) => [visit(key), visit(entry)])];
  if (value instanceof Set) return ['set', [...value].map(visit)];
  if (value instanceof DOMException) return ['exception', value.name, value.message, errorProperties(value, visit)];
  if (value instanceof Error) return ['error', value.name, value.message, errorProperties(value, visit)];
  if (value instanceof ReadableStream) return ['stream', streams.push(byteStream(value)) - 1];
  if (value instanceof ArrayBuffer) {
    if (value.detached) throw dataCloneError();
    return ['buffer', buffers.push(value) - 1];
  }
  if (ArrayBuffer.isView(value)) {
    const name = [...views].find(([, Type]) => Object.getPrototypeOf(value) === Type.prototype)?.[0];
    if (!name) throw dataCloneError();
    return ['view', name, visit(value.buffer), value.byteOffset, value.byteLength];
  }
  const prototype = Object.getPrototypeOf(value);
  if (prototype !== null && prototype !== Object.prototype) throw dataCloneError();
  return ['record', properties(value, visit)];
}

/** Describe a value graph; the transport copies buffers and transfers streams after validation. */
export function encode(value) {
  const nodes = [], buffers = [], streams = [], seen = new Map();
  function visit(value) {
    if (value === null || typeof value === 'string' || typeof value === 'boolean') return value;
    if (typeof value === 'undefined') return ['undefined'];
    if (typeof value === 'bigint') return ['bigint', String(value)];
    if (typeof value === 'number') {
      return Number.isFinite(value) && !Object.is(value, -0) ? value : ['number', Object.is(value, -0) ? '-0' : String(value)];
    }
    if (typeof value !== 'object') throw dataCloneError();
    if (seen.has(value)) return ['ref', seen.get(value)];
    const index = nodes.length;
    seen.set(value, index);
    nodes.push(null);
    nodes[index] = encodeObject(value, visit, buffers, streams);
    return ['ref', index];
  }
  const root = visit(value);
  return { data: JSON.stringify({ root, nodes }), buffers, streams };
}

function setProperties(value, entries, read) {
  for (const [key, entry] of entries) {
    Object.defineProperty(value, key, { value: read(entry), enumerable: true, configurable: true, writable: true });
  }
}

function allocate(node, read, buffers, streams) {
  const [tag, value, extra, offset, length] = node;
  switch (tag) {
    case 'array': return new Array(value);
    case 'record': return {};
    case 'map': return new Map();
    case 'set': return new Set();
    case 'date': return new Date(read(value));
    case 'regexp': return new RegExp(value, extra);
    case 'error': return Object.assign(new Error(extra), { name: value });
    case 'exception': return new DOMException(extra, value);
    case 'buffer': {
      if (!(buffers[value] instanceof ArrayBuffer)) throw dataCloneError();
      return buffers[value];
    }
    case 'stream': {
      if (!(streams[value] instanceof ReadableStream)) throw dataCloneError();
      return streams[value];
    }
    case 'view': {
      const Type = views.get(value);
      if (!Type) throw dataCloneError();
      return new Type(read(extra), offset, length / (Type.BYTES_PER_ELEMENT ?? 1));
    }
    default: throw dataCloneError();
  }
}

function populate(value, node, read) {
  switch (node[0]) {
    case 'array': setProperties(value, node[2], read); break;
    case 'record': setProperties(value, node[1], read); break;
    case 'error': case 'exception': setProperties(value, node[3], read); break;
    case 'map': for (const [key, entry] of node[1]) value.set(read(key), read(entry)); break;
    case 'set': for (const entry of node[1]) value.add(read(entry)); break;
  }
}

/** Reconstruct a graph using buffers and streams owned by this receiver. */
export function decode({ data, buffers = [], streams = [] }) {
  const { root, nodes } = JSON.parse(data), values = new Map(), pending = new Set();
  function read(value) {
    if (!Array.isArray(value)) return value;
    const [tag, entry] = value;
    if (tag === 'undefined') return undefined;
    if (tag === 'bigint') return BigInt(entry);
    if (tag === 'number') return Number(entry);
    if (tag !== 'ref' || !Number.isInteger(entry) || entry < 0 || entry >= nodes.length) throw dataCloneError();
    if (values.has(entry)) return values.get(entry);
    if (pending.has(entry)) throw dataCloneError();
    pending.add(entry);
    const result = allocate(nodes[entry], read, buffers, streams);
    values.set(entry, result);
    populate(result, nodes[entry], read);
    pending.delete(entry);
    return result;
  }
  return read(root);
}
