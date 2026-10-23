import { capabilitiesFor, type Availability, type NativePlatform } from "./capabilities.mjs";
import { platformGlobal } from "./native-protocol.mjs";
import { webview, type WebView } from "./webview.js";

/** The native connection supplied by the host. */
export const nativeConnection = Symbol.for("tokamak.native");

/** Each plugin operation's native platforms, by plugin ID, which the plugins' manifests declare. */
export const nativeAvailability = Symbol.for("tokamak.availability");

export type { Availability, NativePlatform } from "./capabilities.mjs";

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
  /** The platform the host runs on. */
  readonly platform?: NativePlatform;
  call(plugin: string, method: string, payload: Uint8Array, options?: CallOptions): Promise<Uint8Array>;
  /** Subscribes `listener` until the returned function is called. */
  listen(plugin: string, method: string, payload: Uint8Array, listener: NativeListener): () => void;
}

/** The receiving invocation and its native connection. */
export interface TokamakContext {
  waitUntil(promise: Promise<unknown>): void;
  readonly webview: WebView;
  /** The native platform the app runs on, or undefined without a native host. */
  readonly platform: NativePlatform | undefined;
  readonly [nativeConnection]: NativeConnection;
  readonly [nativeAvailability]: Availability;
}

/** The platform the native host names now. */
export function runningPlatform(): NativePlatform | undefined {
  return Reflect.get(globalThis, platformGlobal) as NativePlatform | undefined;
}

/**
 * The native platform the app runs on, or undefined without a native host, such as on Cloudflare.
 * Unlike `ctx.platform`, it is known while modules load.
 */
export const platform = runningPlatform();

function unavailable(): never {
  throw new DOMException("The native device is not connected.", "NotSupportedError");
}

const disconnected: NativeConnection = {
  call: () => Promise.resolve().then(unavailable),
  listen: unavailable,
};

/**
 * The receiving execution context with its native connection, `connection` or the one the host
 * attached to `ctx`, and the plugins' `availability`. Without a connection, native calls reject
 * with `NotSupportedError`.
 */
export function createNativeContext(
  ctx: Pick<TokamakContext, "waitUntil"> & { readonly [nativeConnection]?: NativeConnection },
  connection = ctx[nativeConnection],
  availability: Availability = {},
): TokamakContext {
  const native = connection ?? disconnected;
  return Object.assign(ctx, {
    [nativeConnection]: native,
    [nativeAvailability]: availability,
    platform: native.platform,
    webview: webview(native),
  });
}

/**
 * Whether each operation of `plugin` has an implementation on the platform `ctx` runs on. It does
 * not report permission or promise that a call succeeds.
 */
export function capabilitiesOf(ctx: TokamakContext, plugin: string): Readonly<Record<string, boolean>> {
  return Object.freeze(capabilitiesFor({ [plugin]: ctx[nativeAvailability][plugin] ?? {} }, ctx.platform)[plugin] ?? {});
}
declare global {
  // eslint-disable-next-line @typescript-eslint/no-namespace -- the generated app declarations supply the native bindings.
  namespace Tokamak {
    // eslint-disable-next-line @typescript-eslint/no-empty-object-type -- the app's generated types declare the members.
    interface Env {}
  }
}

