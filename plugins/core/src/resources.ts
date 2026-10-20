import { nativeConnection, type TokamakContext } from "./context.js";
import { decodeJson, encodeJson } from "./codec.js";

const collected = new FinalizationRegistry<() => void>((release) => { release(); });

/**
 * A native object that stays in native memory. The receiving invocation owns it until `close`,
 * the invocation's end, or garbage collection.
 */
export class NativeReference {
  readonly #ctx: TokamakContext;
  readonly #id: string;
  #release: (() => void) | undefined;

  private constructor(ctx: TokamakContext, id: string, release: () => void) {
    this.#ctx = ctx;
    this.#id = id;
    this.#release = release;
    collected.register(this, release, this);
  }

  /** Retains the object that the native subscription `method` of `plugin` holds open. */
  static async acquire(ctx: TokamakContext, plugin: string, method: string, payload: Uint8Array): Promise<NativeReference> {
    let release = () => {};
    try {
      // The subscription's listeners never hold the reference, so garbage collection can release it.
      const id = await new Promise<Uint8Array>((resolve, reject) => {
        let settled = false;
        // Later replies are ignored: a released object fails where it is used.
        release = ctx[nativeConnection].listen(plugin, method, payload, (value, done) => {
          if (settled) return;
          settled = true;
          if (done) reject(new DOMException("The native object was not retained", "InvalidStateError"));
          else resolve(value);
        }, (error) => {
          if (settled) return;
          settled = true;
          reject(error);
        });
      });
      return new NativeReference(ctx, decodeJson<string>(id), release);
    } catch (error) {
      release();
      throw error;
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
