import { nativeConnection, type CallOptions, type NativeConnection, type Overflow, type TokamakContext } from "./context.js";
import { byteStream } from "./streams.js";
import { decodeJson, encodeJson } from "./codec.js";
import { NativeReference } from "./resources.js";

export type { TokamakContext } from "./worker.js";
export type { CallOptions, Overflow } from "./context.js";
export type { JsonValue } from "./codec.js";
export { decodeJson, encodeJson } from "./codec.js";
export { NativeReference } from "./resources.js";

/** Native endpoint calls owned by the receiving invocation. */
export abstract class Plugin {
  private readonly connection: NativeConnection;

  protected constructor(private readonly ctx: TokamakContext, private readonly pluginId: string) {
    const connection = Reflect.get(ctx, nativeConnection) as NativeConnection | undefined;
    if (!connection) throw new TypeError("A Tokamak invocation context is required");
    this.connection = connection;
  }

  protected async call<T>(method: string, value: unknown = null, options?: CallOptions): Promise<T> {
    return decodeJson<T>(await this.callBytes(method, encodeJson(value), options));
  }

  protected callBytes(method: string, payload: Uint8Array, options?: CallOptions): Promise<Uint8Array> {
    return this.connection.call(this.pluginId, method, payload, options);
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
  protected stream(method: string, value: unknown = null, overflow: Overflow = "error"): ReadableStream<Uint8Array> {
    return byteStream((next, error, close) => this.connection.listen(this.pluginId, method, encodeJson(value), {
      next(payload, done) {
        const line = new Uint8Array(payload.byteLength + 1);
        line.set(payload);
        line[payload.byteLength] = 10;
        next(line);
        if (done) close();
      },
      error,
      overflow,
    }), overflow);
  }
}
