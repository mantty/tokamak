import { nativeConnection, type NativeConnection, type TokamakContext } from "./context.js";
import { byteStream } from "./streams.js";
import { decodeJson, encodeJson } from "./codec.js";

export type { TokamakContext } from "./worker.js";
export type { JsonValue } from "./codec.js";
export { decodeJson, encodeJson } from "./codec.js";

/** Native endpoint calls owned by the receiving invocation. */
export abstract class Plugin {
  private readonly connection: NativeConnection;

  protected constructor(ctx: TokamakContext, private readonly pluginId: string) {
    const connection = Reflect.get(ctx, nativeConnection) as NativeConnection | undefined;
    if (!connection) throw new TypeError("A Tokamak invocation context is required");
    this.connection = connection;
  }

  protected async call<T>(method: string, value: unknown = null): Promise<T> {
    return decodeJson<T>(await this.callBytes(method, encodeJson(value)));
  }

  protected callBytes(method: string, payload: Uint8Array): Promise<Uint8Array> {
    return this.connection.call(this.pluginId, method, payload);
  }

  /** A stream of newline-delimited JSON values, consumed with decodeValues. */
  protected stream(method: string, value: unknown = null): ReadableStream<Uint8Array> {
    return byteStream((next, error, close) => this.connection.listen(
      this.pluginId, method, encodeJson(value),
      (payload, done) => {
        const line = new Uint8Array(payload.byteLength + 1);
        line.set(payload);
        line[payload.byteLength] = 10;
        next(line);
        if (done) close();
      },
      error,
    ));
  }
}
