import { capabilitiesOf, nativeConnection, type CallOptions, type Overflow, type TokamakContext } from "./context.js";
import { byteStream, jsonLine, type JsonStream } from "./streams.js";
import { decodeJson, encodeJson } from "./codec.js";
import { NativeReference } from "./resources.js";

export type { TokamakContext } from "./worker.js";
export type { CallOptions, NativePlatform, Overflow } from "./context.js";
export { capabilitiesOf } from "./context.js";
export type { JsonStream } from "./streams.js";
export type { JsonValue } from "./codec.js";
export { decodeJson, encodeJson } from "./codec.js";
export { NativeReference } from "./resources.js";

/** Whether the app is on screen. */
export type LifecycleStage = "foreground" | "background";

/**
 * The app's stage now, which can differ from the last lifecycle event delivered. Rejects with
 * `NotSupportedError` without a native host.
 */
export async function lifecycleStage(ctx: TokamakContext): Promise<LifecycleStage> {
  return decodeJson<LifecycleStage>(await ctx[nativeConnection].call("tokamak", "lifecycleStage", encodeJson(null)));
}

/** Native endpoint calls owned by the receiving invocation. */
export abstract class Plugin {
  protected constructor(protected readonly ctx: TokamakContext, private readonly pluginId: string) {
    if (!Reflect.has(ctx, nativeConnection)) throw new TypeError("A Tokamak invocation context is required");
  }

  protected async call<T>(method: string, value: unknown = null, options?: CallOptions): Promise<T> {
    return decodeJson<T>(await this.callBytes(method, encodeJson(value), options));
  }

  /**
   * Whether each operation has an implementation on the platform the app runs on. It does not
   * report permission or promise that a call succeeds.
   */
  get capabilities(): Readonly<Record<string, boolean>> {
    return capabilitiesOf(this.ctx, this.pluginId);
  }

  protected callBytes(method: string, payload: Uint8Array, options?: CallOptions): Promise<Uint8Array> {
    return this.ctx[nativeConnection].call(this.pluginId, method, payload, options);
  }

  /** Retains the native object that the subscription `method` holds open, without copying it. */
  protected resource(method: string, value: unknown = null): Promise<NativeReference> {
    return NativeReference.acquire(this.ctx, this.pluginId, method, encodeJson(value));
  }

  /** The ID that passes `reference` to this plugin's native endpoint, which finds the same object. */
  protected reference(reference: NativeReference): string {
    return reference.idFor(this.ctx);
  }

  /**
   * A stream of newline-delimited JSON values, consumed with decodeValues. `overflow` says what
   * happens to values a slow reader has not taken: ending the stream, or dropping the oldest.
   */
  protected stream<T>(method: string, value: unknown = null, overflow: Overflow = "error"): JsonStream<T> {
    return byteStream((next, error, close) => this.ctx[nativeConnection].listen(this.pluginId, method, encodeJson(value), {
      next(payload, done) {
        next(jsonLine(payload));
        if (done) close();
      },
      error,
      overflow,
    }), overflow);
  }
}
