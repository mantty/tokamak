import { dispatch as dispatchEvent, type Dispatched } from "./registry.js";
import { createNativeContext, type Availability, type NativeConnection, type NativePlatform, type Overflow, type TokamakContext } from "./context.js";
import { encodeJson, decodeJson } from "./codec.js";
import { nativeError, type NativeFailure } from "./native-protocol.mjs";

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

/** A reply to a request: its value or failure, and whether it is the last. */
export interface Answer {
  value?: unknown;
  error?: NativeFailure;
  done?: boolean;
}

/** A request as `answer` sees it, with its decoded payload. */
export interface Received {
  id: number;
  type: string;
  plugin?: string;
  method?: string;
  arguments: unknown;
}

/** An invocation with an inspectable native endpoint. */
export class FakeNativeTransport {
  readonly sent: SentRequest[] = [];
  private nextId = 1;
  private readonly pending = new Map<number, Reply>();
  private answering: ((request: Received) => Answer | undefined) | undefined;
  readonly ctx: TokamakContext;

  /** An invocation on `platform`, whose plugins' operations have `availability`. */
  constructor(platform: NativePlatform = "ios", availability: Availability = {}) {
    this.ctx = createNativeContext({ waitUntil: () => undefined }, this.connection(platform), availability);
  }

  private connection(platform: NativePlatform): NativeConnection {
    return {
      platform,
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
    };
  }

  private request(request: Omit<SentRequest, "id">, reply: Reply): number {
    const id = this.nextId++;
    this.sent.push({ ...request, id, payload: request.payload?.slice() });
    this.pending.set(id, reply);
    const { answering } = this;
    if (answering) {
      queueMicrotask(() => {
        const { type, plugin, method, payload } = request;
        const answer = answering({ id, type, plugin, method, arguments: payload?.byteLength ? decodeJson(payload) : undefined });
        if (answer) this.deliver(id, answer);
      });
    }
    return id;
  }

  /**
   * Answers each later request, as it arrives, with what `answer` returns for it. Undefined leaves
   * the request pending, for `respond` or `deliver`.
   */
  answer(answer: (request: Received) => Answer | undefined): void {
    this.answering = answer;
  }

  /** Replies to the pending request `id`. A reply is the last unless `done` is false. */
  deliver(id: number, { value, error, done = true }: Answer): void {
    const reply = this.pending.get(id);
    if (done) this.pending.delete(id);
    reply?.(encodeJson(value), error && nativeError(error), done);
  }

  private cancel(id: number): void {
    if (this.pending.delete(id)) this.sent.push({ type: "cancel", id });
  }

  lastRequest(): Record<string, unknown> {
    const { type, plugin, method, payload } = this.sent.at(-1) ?? {};
    return { type, plugin, method, arguments: payload ? decodeJson(payload) : undefined };
  }

  respond(reply: { value?: unknown; error?: NativeFailure }, done = true): void {
    this.respondBytes(encodeJson(reply.value), done, reply.error && nativeError(reply.error));
  }

  respondBytes(payload: Uint8Array, done = true, error?: DOMException): void {
    const id = this.sent.at(-1)?.id;
    if (id === undefined) throw new Error("No native request is pending");
    const reply = this.pending.get(id);
    if (done) this.pending.delete(id);
    reply?.(payload.slice(), error, done);
  }
}

export function connectNative(platform?: NativePlatform, availability?: Availability): FakeNativeTransport {
  return new FakeNativeTransport(platform, availability);
}

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
