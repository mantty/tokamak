import { dispatch as dispatchEvent, type Dispatched } from "./registry.js";
import { createNativeContext, nativeConnection, type TokamakContext } from "./context.js";
import { encodeJson, decodeJson } from "./codec.js";

type Reply = (payload: Uint8Array, error: DOMException | undefined, done: boolean) => void;

/** An invocation with an inspectable native endpoint. */
export class FakeNativeTransport {
  readonly sent: { type: string; id: number; plugin?: string; method?: string; payload?: Uint8Array }[] = [];
  private nextId = 1;
  private readonly pending = new Map<number, Reply>();
  readonly ctx: TokamakContext = {
    waitUntil: () => undefined,
    [nativeConnection]: {
      call: (plugin, method, payload) => new Promise((resolve, reject) => {
        this.request("call", plugin, method, payload, (value, error) => { if (error) reject(error); else resolve(value); });
      }),
      listen: (plugin, method, payload, next, error) => {
        const id = this.request("subscribe", plugin, method, payload, (value, failure, done) => {
          if (failure) error(failure, done);
          else next(value, done);
        });
        return () => {
          if (this.pending.delete(id)) this.sent.push({ type: "cancel", id });
        };
      },
    },
  };

  private request(type: string, plugin: string, method: string, payload: Uint8Array, reply: Reply): number {
    const id = this.nextId++;
    this.sent.push({ type, id, plugin, method, payload: payload.slice() });
    this.pending.set(id, reply);
    return id;
  }

  lastRequest(): Record<string, unknown> {
    const { type, plugin, method, payload } = this.sent.at(-1) ?? {};
    return { type, plugin, method, arguments: payload ? decodeJson(payload) : undefined };
  }

  respond(reply: { value?: unknown; error?: { name: string; message: string } }, done = true): void {
    this.respondBytes(encodeJson(reply.value), done, reply.error && new DOMException(reply.error.message, reply.error.name));
  }

  respondBytes(payload: Uint8Array, done = true, error?: DOMException): void {
    const id = this.sent.at(-1)?.id;
    if (id === undefined) throw new Error("No native request is pending");
    const reply = this.pending.get(id);
    if (done) this.pending.delete(id);
    reply?.(payload.slice(), error, done);
  }
}

export function connectNative(): FakeNativeTransport { return new FakeNativeTransport(); }

/**
 * Runs the listeners of the event `name` as the runtime does, returning the reply, the events
 * that have listeners, and what listeners threw, which the runtime reports with `reportError`.
 */
export async function dispatch(
  name: string,
  event: unknown,
  env: unknown = {},
  ctx: unknown = createNativeContext({ waitUntil: () => undefined }),
): Promise<Dispatched & { reported: unknown[] }> {
  const reported: unknown[] = [];
  const previous = Reflect.get(globalThis, "reportError") as unknown;
  globalThis.reportError = (error: unknown) => reported.push(error);
  try {
    return { ...(await dispatchEvent(name, event, env, ctx)), reported };
  } finally {
    Reflect.set(globalThis, "reportError", previous);
  }
}
