import { webview, type WebView } from "./webview.js";

/** The native connection supplied by the host. */
export const nativeConnection = Symbol.for("tokamak.native");

export interface NativeConnection {
  call(plugin: string, method: string, payload: Uint8Array): Promise<Uint8Array>;
  listen(
    plugin: string,
    method: string,
    payload: Uint8Array,
    next: (value: Uint8Array, done: boolean) => void,
    error: (error: DOMException, done: boolean) => void,
  ): () => void;
}

/** The receiving invocation and its native connection. */
export interface TokamakContext {
  waitUntil(promise: Promise<unknown>): void;
  readonly webview: WebView;
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
export function createNativeContext(
  ctx: Pick<TokamakContext, "waitUntil">,
  connection = Reflect.get(globalThis, nativeConnection) as NativeConnection | undefined,
): TokamakContext {
  const native = connection ?? disconnected;
  return Object.assign(ctx, { [nativeConnection]: native, webview: webview(native) });
}
declare global {
  // eslint-disable-next-line @typescript-eslint/no-namespace -- the generated app declarations supply the native bindings.
  namespace Tokamak {
    // eslint-disable-next-line @typescript-eslint/no-empty-object-type -- the app's generated types declare the members.
    interface Env {}
  }
}

