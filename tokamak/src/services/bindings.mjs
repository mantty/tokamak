import { serviceStreamCodec } from './streams.mjs';
import { decodeRequest, encodeResponse } from '../rpc/http.mjs';
import { fetchService } from './http.mjs';
import { encode, decode, dataCloneError } from '../rpc/values.mjs';
import { WorkerEntrypoint, ServiceStub } from '../builtins/cloudflare-workers.mjs';

export function installServices(names, dispatch) {
  const call = (name, method, message) => dispatch(name, method, message, serviceStreamCodec);
  for (const name of names) {
    if (Object.hasOwn(globalThis.__tokamak_env, name)) throw new TypeError(`Duplicate service binding ${name}`);
    globalThis.__tokamak_env[name] = new Proxy(Object.create(ServiceStub.prototype), {
      get(target, method) {
        if (method === 'fetch') return (input, init) => fetchService(call, name, input, init);
        if (method === 'then' || typeof method === 'symbol') return undefined;
        return async (...args) => {
          const reply = await call(name, method, encode(args)).result;
          const value = decode(reply.value);
          if (reply.failed) throw value;
          return value;
        };
      },
    });
  }
}

function operation(worker, method) {
  if ([
    'event', 'fetch', 'queue', 'scheduled', 'email', 'tail', 'trace', 'tailStream', 'test',
    'constructor', 'connect', 'then', 'dup', 'alarm', 'webSocketMessage', 'webSocketClose', 'webSocketError', '__tokamakRpc',
  ].includes(method)) {
    throw new TypeError(`Reserved service method ${method}`);
  }
  let prototype = Object.getPrototypeOf(worker);
  while (prototype && prototype !== WorkerEntrypoint.prototype && prototype !== Object.prototype) {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, method);
    if (descriptor) {
      if (typeof descriptor.value !== 'function') break;
      return descriptor.value;
    }
    prototype = Object.getPrototypeOf(prototype);
  }
  throw new TypeError(`Service method ${method} is not a public prototype method`);
}

export async function invokeService(exports, entrypoint, method, argumentsMessage) {
  try {
    const Type = exports[entrypoint];
    if (method === 'fetch') {
      const worker = typeof Type === 'function' ? new Type(globalThis.__tokamak_context, globalThis.__tokamak_env) : Type;
      if (typeof worker?.fetch !== 'function') throw new TypeError(`Service entrypoint ${entrypoint} has no fetch handler`);
      const request = decodeRequest(decode(argumentsMessage));
      const response = await worker.fetch(request, globalThis.__tokamak_env, globalThis.__tokamak_context);
      return { failed: false, value: encode(encodeResponse(response)) };
    }
    if (typeof Type !== 'function' || !(Type.prototype instanceof WorkerEntrypoint)) throw new TypeError(`Service entrypoint ${entrypoint} must extend WorkerEntrypoint`);
    const worker = new Type(globalThis.__tokamak_context, globalThis.__tokamak_env);
    const result = await operation(worker, method).apply(worker, decode(argumentsMessage));
    return { failed: false, value: encode(result) };
  } catch (error) {
    try { return { failed: true, value: encode(error) }; }
    catch { return { failed: true, value: encode(dataCloneError()) }; }
  }
}
