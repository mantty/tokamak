import { nativeConnection, type TokamakContext } from "./context.js";
import { decodeJson, encodeJson } from "./codec.js";

const collected = new FinalizationRegistry<() => void>((release) => { release(); });

/** The invocation that owns `reference`. */
export let ownerOf: (reference: NativeReference) => TokamakContext;

/**
 * A native object that stays in native memory. The receiving invocation owns it until `close`,
 * the invocation's end, or garbage collection.
 */
export class NativeReference {
  readonly #ctx: TokamakContext;
  readonly #id: string;
  #release: (() => void) | undefined;

  static {
    ownerOf = (reference) => reference.#ctx;
  }

  private constructor(ctx: TokamakContext, id: string, release: () => void) {
    this.#ctx = ctx;
    this.#id = id;
    this.#release = release;
    collected.register(this, release, this);
  }

  /**
   * Retains the object that the native subscription `method` of `plugin` holds open: its first
   * value is the object's ID, and later values go to `next`.
   */
  static async acquire(
    ctx: TokamakContext,
    plugin: string,
    method: string,
    payload: Uint8Array,
    next: (value: Uint8Array) => void = () => {},
  ): Promise<NativeReference> {
    let release = () => {};
    try {
      // The subscription's listeners never hold the reference, so garbage collection can release it.
      const id = await new Promise<Uint8Array>((resolve, reject) => {
        let settled = false;
        release = ctx[nativeConnection].listen(plugin, method, payload, {
          next(value, done) {
            if (settled) {
              if (!done) next(value);
              return;
            }
            settled = true;
            if (done) reject(new DOMException("The native object was not retained", "InvalidStateError"));
            else resolve(value);
          },
          // A later failure ends the subscription: the released object fails where it is used.
          error(error) {
            if (settled) return;
            settled = true;
            reject(error);
          },
        });
      });
      return new NativeReference(ctx, decodeJson<string>(id), release);
    } catch (error) {
      release();
      throw error;
    }
  }

  /** The object retained as `id` in the scope of the invocation `ctx`, which ends with it. */
  static scoped(ctx: TokamakContext, id: string): NativeReference {
    return new NativeReference(ctx, id, () => {
      // A failed release changes nothing: the invocation's end has released the object.
      ctx[nativeConnection].call("tokamak", "resource.release", encodeJson(id)).catch(() => undefined);
    });
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
    collected.unregister(this);
    release();
  }

  /** The ID a plugin's native endpoint uses to find the object, within the invocation `ctx` that owns it. */
  idFor(ctx: TokamakContext): string {
    if (!this.#release) throw new DOMException("The native reference is closed", "InvalidStateError");
    if (ctx !== this.#ctx) throw new DOMException("The native reference belongs to another invocation", "InvalidStateError");
    return this.#id;
  }
}
