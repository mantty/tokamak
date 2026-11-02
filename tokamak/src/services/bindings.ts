import type { JsMessage, JsReply, ServiceHost } from 'tokamak:host';
import { serviceStreamCodec, failure, replyValue } from './streams.ts';
import { decodeRequest, encodeResponse } from '../rpc/http.ts';
import { fetchService, type ServiceCaller } from './http.ts';
import { encode, decode, list } from '../rpc/values.ts';
import { WorkerEntrypoint, ServiceStub } from '../builtins/cloudflare-workers.ts';
import { isFunction, propertyOf, type AnyFunction } from '../globals/objects.ts';

export function installServices(names: string[], host: ServiceHost): void {
  const call: ServiceCaller = (name, method, message) => host.call(name, method, message, serviceStreamCodec);
  for (const name of names) {
    if (Object.hasOwn(globalThis.__tokamak_env, name)) throw new TypeError(`Duplicate service binding ${name}`);
    const stub: object = Object.create(ServiceStub.prototype);
    globalThis.__tokamak_env[name] = new Proxy(stub, {
      get(_target, method) {
        if (method === 'fetch') return (input: RequestInfo | URL, init?: RequestInit) => fetchService(call, name, input, init);
        if (method === 'then' || typeof method === 'symbol') return undefined;
        return async (...args: unknown[]) => replyValue(await call(name, method, encode(args)).reply);
      },
    });
  }
}

const reservedMethods = [
  'event', 'fetch', 'queue', 'scheduled', 'email', 'tail', 'trace', 'tailStream', 'test',
  'constructor', 'connect', 'then', 'dup', 'alarm', 'webSocketMessage', 'webSocketClose', 'webSocketError', '__tokamakRpc',
];

function operation(worker: object, method: string): AnyFunction {
  if (reservedMethods.includes(method)) throw new TypeError(`Reserved service method ${method}`);
  let prototype: object | null = Object.getPrototypeOf(worker);
  while (prototype && prototype !== WorkerEntrypoint.prototype && prototype !== Object.prototype) {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, method);
    if (descriptor) {
      const value: unknown = descriptor.value;
      if (!isFunction(value)) break;
      return value;
    }
    prototype = Object.getPrototypeOf(prototype);
  }
  throw new TypeError(`Service method ${method} is not a public prototype method`);
}

// The Worker's `entrypoint` export, constructed when it is a class.
function serviceWorker(Type: unknown): unknown {
  return typeof Type === 'function' ? Reflect.construct(Type, [globalThis.__tokamak_context, globalThis.__tokamak_env]) : Type;
}

export async function invokeService(exports: object, entrypoint: string, method: string, message: JsMessage): Promise<JsReply> {
  try {
    const Type: unknown = Reflect.get(exports, entrypoint);
    if (method === 'fetch') {
      const worker = serviceWorker(Type);
      const fetch = propertyOf(worker, 'fetch');
      if (!isFunction(fetch)) throw new TypeError(`Service entrypoint ${entrypoint} has no fetch handler`);
      const request = decodeRequest(decode(message));
      const response = await Reflect.apply(fetch, worker, [request, globalThis.__tokamak_env, globalThis.__tokamak_context]);
      return { failed: false, value: encode(encodeResponse(response)) };
    }
    if (typeof Type !== 'function' || !(Type.prototype instanceof WorkerEntrypoint)) throw new TypeError(`Service entrypoint ${entrypoint} must extend WorkerEntrypoint`);
    const worker: object = Reflect.construct(Type, [globalThis.__tokamak_context, globalThis.__tokamak_env]);
    const result: unknown = await Reflect.apply(operation(worker, method), worker, list(decode(message)));
    return { failed: false, value: encode(result) };
  } catch (error) {
    return failure(error);
  }
}
