import { dispatch as dispatchEvent, type Dispatched } from "./registry.js";
import { createNativeContext, type Overflow, type TokamakContext } from "./context.js";
import { encodeJson, decodeJson } from "./codec.js";

type Reply = (payload: Uint8Array, error: DOMException | undefined, done: boolean) => void;

/** A request the native endpoint received. */
export interface SentRequest {
  type: string;
  id: number;
  plugin?: string;
  method?: string;
  payload?: Uint8Array;
  /** A call's body. */
  body?: ReadableStream<Uint8Array>;
  /** A subscription's overflow policy. */
  overflow?: Overflow;
}

/** An invocation with an inspectable native endpoint. */
export class FakeNativeTransport {
  readonly sent: SentRequest[] = [];
  private nextId = 1;
  private readonly pending = new Map<number, Reply>();
  readonly ctx: TokamakContext = createNativeContext({ waitUntil: () => undefined }, {
    call: (plugin, method, payload, { body, signal } = {}) => new Promise((resolve, reject) => {
      signal?.throwIfAborted();
      const id = this.request({ type: "call", plugin, method, payload, body }, (value, error) => {
        if (error) reject(error);
        else resolve(value);
      });
      signal?.addEventListener("abort", () => {
        this.cancel(id);
        reject(signal.reason as Error);
      }, { once: true });
    }),
    listen: (plugin, method, payload, { next, error, overflow = "error" }) => {
      const id = this.request({ type: "subscribe", plugin, method, payload, overflow }, (value, failure, done) => {
        if (failure) error(failure, done);
        else next(value, done);
      });
      return () => { this.cancel(id); };
    },
  });

  private request(request: Omit<SentRequest, "id">, reply: Reply): number {
    const id = this.nextId++;
    this.sent.push({ ...request, id, payload: request.payload?.slice() });
    this.pending.set(id, reply);
    return id;
  }

  private cancel(id: number): void {
    if (this.pending.delete(id)) this.sent.push({ type: "cancel", id });
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
