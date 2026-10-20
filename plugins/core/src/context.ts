import { webview, type WebView } from "./webview.js";

/** The native connection supplied by the host. */
export const nativeConnection = Symbol.for("tokamak.native");

/** A native call's request body and cancellation. */
export interface CallOptions {
  /** Bytes the native endpoint reads one chunk at a time, as it needs them. */
  readonly body?: ReadableStream<Uint8Array>;
  /** Cancels the call, which then rejects with the signal's reason. */
  readonly signal?: AbortSignal;
}

/** What a subscription does with a value that arrives while its buffer is full. */
export type Overflow = "error" | "dropOldest";

/** Receives a native subscription's values and failures. */
export interface NativeListener {
  readonly next: (value: Uint8Array, done: boolean) => void;
  readonly error: (error: DOMException, done: boolean) => void;
  /** What happens to values a slow reader has not taken; `"error"` by default. */
  readonly overflow?: Overflow;
}

export interface NativeConnection {
  call(plugin: string, method: string, payload: Uint8Array, options?: CallOptions): Promise<Uint8Array>;
  /** Subscribes `listener` until the returned function is called. */
  listen(plugin: string, method: string, payload: Uint8Array, listener: NativeListener): () => void;
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

/**
 * The receiving execution context with its native connection: `connection`, or the one the host
 * attached to `ctx`. Without either, native calls reject with `NotSupportedError`.
 */
export function createNativeContext(
  ctx: Pick<TokamakContext, "waitUntil"> & { readonly [nativeConnection]?: NativeConnection },
  connection = ctx[nativeConnection],
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

