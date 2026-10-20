/** The native connection supplied by the host. */
export const nativeConnection = Symbol.for("tokamak.native");

export interface NativeConnection {
  call(plugin: string, method: string, arguments_?: unknown): Promise<unknown>;
  listen(
    plugin: string,
    method: string,
    arguments_: unknown,
    next: (value: unknown) => void,
    error: (error: DOMException) => void,
  ): () => void;
}

/** The receiving invocation and its native connection. */
export interface TokamakContext {
  waitUntil(promise: Promise<unknown>): void;
  readonly [nativeConnection]: NativeConnection;
}

function unavailable(): never {
  throw new DOMException("The native device is not connected.", "NotSupportedError");
}

const disconnected: NativeConnection = {
  call: () => Promise.resolve().then(unavailable),
  listen: unavailable,
};

/** Attaches the host's connection to the receiving execution context. */
export function createNativeContext(ctx: Pick<TokamakContext, "waitUntil">): TokamakContext {
  const connection = Reflect.get(globalThis, nativeConnection) as NativeConnection | undefined;
  return Object.assign(ctx, { [nativeConnection]: connection ?? disconnected });
}
declare global {
  // eslint-disable-next-line @typescript-eslint/no-namespace -- `wrangler types` declares the Worker's bindings here.
  namespace Cloudflare {
    // eslint-disable-next-line @typescript-eslint/no-empty-object-type -- the app's `wrangler types` declares the members.
    interface Env {}
  }
}

