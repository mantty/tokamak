import { nativeConnection, type NativePlatform, type Overflow, type TokamakContext } from "./context.js";
import { byteStream, jsonLine, type JsonStream } from "./streams.js";
import { decodeJson, encodeJson } from "./codec.js";
import { NativeReference } from "./resources.js";

/**
 * A plugin's native methods on `platforms`, which its generated platform APIs call. Off those
 * platforms, calls reject with `NotSupportedError`. Aborting `signal` cancels the calls.
 */
export class NativeEndpoint {
  readonly #ctx: TokamakContext;
  readonly #plugin: string;
  readonly #platforms: readonly NativePlatform[];
  readonly #signal: AbortSignal | undefined;

  constructor(ctx: TokamakContext, plugin: string, platforms: readonly NativePlatform[], signal?: AbortSignal) {
    if (!Reflect.has(ctx, nativeConnection)) throw new TypeError("A Tokamak invocation context is required");
    this.#ctx = ctx;
    this.#plugin = plugin;
    this.#platforms = platforms;
    this.#signal = signal;
  }

  /** Calls `method` with `args` encoded as JSON, resolving to its decoded result. */
  async call<T>(method: string, args: object = {}, body?: ReadableStream<Uint8Array>): Promise<T> {
    return decodeJson<T>(await this.callBytes(method, encodeJson(args), body));
  }

  /** Calls `method` with `payload`, resolving to the bytes it replies with. */
  async callBytes(method: string, payload: Uint8Array, body?: ReadableStream<Uint8Array>): Promise<Uint8Array> {
    return this.#connection(method).call(this.#plugin, method, payload, { body, signal: this.#signal });
  }

  /**
   * The values `method` sends, as newline-delimited JSON for `decodeValues`. `overflow` says what
   * happens to values a slow reader has not taken: ending the stream, or dropping the oldest.
   */
  stream<T>(method: string, args: object = {}, overflow: Overflow = "error"): JsonStream<T> {
    return byteStream((next, error, close) => this.#connection(method).listen(this.#plugin, method, encodeJson(args), {
      next(payload, done) {
        // A subscription that ends after its last value sends no value with its end.
        if (payload.byteLength) next(jsonLine(payload));
        if (done) close();
      },
      error,
      overflow,
    }), overflow);
  }

  /** Retains the native object `method` returns, without copying it. */
  async resource(method: string, args: object = {}): Promise<NativeReference> {
    this.#connection(method);
    return NativeReference.acquire(this.#ctx, this.#plugin, method, encodeJson(args));
  }

  /** The ID that passes `reference` to this plugin's native code, which finds the same object. */
  reference(reference: NativeReference): string {
    return reference.idFor(this.#ctx);
  }

  /** The native connection, where `method` runs on the platform the app runs on. */
  #connection(method: string) {
    const { platform } = this.#ctx;
    if (platform === undefined || !this.#platforms.includes(platform)) {
      throw new DOMException(`${this.#plugin}.${method} is not supported ${platform ? `on ${platform}` : "without a native host"}`, "NotSupportedError");
    }
    return this.#ctx[nativeConnection];
  }
}

type PlatformApiFactory = (ctx: TokamakContext, signal?: AbortSignal) => object;

/** A plugin's generated platform APIs, by platform family. */
export interface PlatformApis {
  readonly apple?: PlatformApiFactory;
  readonly android?: PlatformApiFactory;
}

/** Any of the APIs `Apis` creates. */
type PlatformApi<Apis extends PlatformApis> = {
  [Family in keyof Apis]: Apis[Family] extends PlatformApiFactory ? ReturnType<Apis[Family]> : never;
}[keyof Apis];

const families: Partial<Record<NativePlatform, keyof PlatformApis>> = { ios: "apple", macos: "apple", android: "android" };

/**
 * The API in `apis` for the platform `ctx` runs on: `apple` on iOS and macOS, `android` on Android.
 * Elsewhere it is the first, whose calls reject with `NotSupportedError`. Aborting `signal`
 * cancels its calls.
 */
export function platformApi<Apis extends PlatformApis>(ctx: TokamakContext, apis: Apis, signal?: AbortSignal): PlatformApi<Apis> {
  const family = ctx.platform && families[ctx.platform];
  const create = (family && apis[family]) ?? Object.values(apis)[0] as PlatformApiFactory;
  return create(ctx, signal) as PlatformApi<Apis>;
}
