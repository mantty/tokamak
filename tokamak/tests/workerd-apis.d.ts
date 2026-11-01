// workerd APIs that workers-types does not declare, or declares more narrowly
// than workerd implements them.
declare namespace Cloudflare {
  // The binding the test Workers' configuration gives them.
  interface Env {
    FLAG: string;
  }
}

// The class of an RPC result, which no Worker can construct or call.
interface RpcResultClass {
  new (): never;
  (): never;
  readonly name: string;
  readonly prototype: { then: Function; catch: Function; finally: Function; [Symbol.dispose]?: Function };
}

interface Crypto {
  // Web Crypto fills a Uint8ClampedArray too.
  getRandomValues<T extends Int8Array | Uint8Array | Uint8ClampedArray | Int16Array | Uint16Array | Int32Array | Uint32Array | BigInt64Array | BigUint64Array>(buffer: T): T;
}

interface Performance {
  // A measure's start and end may name marks.
  measure(measureName: string, measureOptions: { start?: number | string; end?: number | string; duration?: number; detail?: unknown }): PerformanceMeasure;
}

// The global scope is an EventTarget, which takes events of any type.
declare function addEventListener(type: string, handler: EventListenerOrEventListenerObject, options?: EventTargetAddEventListenerOptions | boolean): void;
declare function removeEventListener(type: string, handler: EventListenerOrEventListenerObject, options?: EventTargetEventListenerOptions | boolean): void;
declare function dispatchEvent(event: Event): boolean;

declare namespace CloudflareWorkersModule {
  export const RpcPromise: RpcResultClass;
  export const RpcProperty: RpcResultClass;
}

declare module "cloudflare:sockets" {
  // workerd exports it, though calls throw until it enables the internal client.
  export function internalNewHttpClient(socket: Socket): never;
}

declare module "cloudflare:node" {
  // Node's servers, whose address() @types/node declares as possibly a string.
  export function httpServerHandler(server: import("node:http").Server): ExportedHandler;
}
