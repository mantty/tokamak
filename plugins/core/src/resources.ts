import { nativeConnection, type TokamakContext } from "./context.js";
import { decodeJson, encodeJson } from "./codec.js";

// Workers have FinalizationRegistry from compatibility date 2025-05-05 unless `disable_weak_ref` is set.
const collected = typeof FinalizationRegistry === "function"
  ? new FinalizationRegistry<() => void>((release) => { release(); })
  : undefined;

/**
 * A native object that stays in native memory. The receiving invocation owns it until `close`,
 * the invocation's end, or garbage collection where the runtime has `FinalizationRegistry`.
 */
export class NativeReference {
  readonly #ctx: TokamakContext;
  readonly #id: string;
  #release: (() => void) | undefined;

  private constructor(ctx: TokamakContext, id: string, release: () => void) {
    this.#ctx = ctx;
    this.#id = id;
    this.#release = release;
    collected?.register(this, release, this);
  }

  /**
   * Retains the object that the native subscription `method` of `plugin` holds open: its first
   * value is the object's ID. Aborting `signal` before then releases the object, rejecting with
   * the signal's reason.
   */
  static async acquire(ctx: TokamakContext, plugin: string, method: string, payload: Uint8Array, signal?: AbortSignal): Promise<NativeReference> {
    signal?.throwIfAborted();
    let release = () => {};
    let abort = () => {};
    try {
      // The subscription's listeners never hold the reference, so garbage collection can release it.
      const id = await new Promise<Uint8Array>((resolve, reject) => {
        abort = () => { reject(signal?.reason as Error); };
        signal?.addEventListener("abort", abort, { once: true });
        release = ctx[nativeConnection].listen(plugin, method, payload, {
          next(value, done) {
            if (done) reject(new DOMException("The native object was not retained", "InvalidStateError"));
            else resolve(value);
          },
          // A later failure ends the subscription: the released object fails where it is used.
          error: reject,
        });
      });
      return new NativeReference(ctx, decodeJson<string>(id), release);
    } catch (error) {
      release();
      throw error;
    } finally {
      signal?.removeEventListener("abort", abort);
    }
  }

  /** Copies the current contents of a native byte buffer. Other native objects reject with `TypeError`. */
  async bytes(): Promise<Uint8Array> {
    return this.#ctx[nativeConnection].call("tokamak", "resource.read", encodeJson(this.idFor(this.#ctx)));
  }

  /** Releases the native object. Later calls do nothing. */
  close(): void {
    const release = this.#release;
    if (!release) return;
    this.#release = undefined;
    collected?.unregister(this);
    release();
  }

  /** The ID a plugin's native endpoint uses to find the object, within the invocation `ctx` that owns it. */
  idFor(ctx: TokamakContext): string {
    if (!this.#release) throw new DOMException("The native reference is closed", "InvalidStateError");
    if (ctx !== this.#ctx) throw new DOMException("The native reference belongs to another invocation", "InvalidStateError");
    return this.#id;
  }
}
