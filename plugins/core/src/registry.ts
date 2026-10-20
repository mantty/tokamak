import { decodeJson, encodeJson } from "./codec.js";

/** A registered listener. */
export type RegisteredListener = (event: unknown, env: unknown, ctx: unknown) => unknown;

/** How an event's data and reply travel as bytes. An empty reply is no reply. */
export interface EventCodec<T, Reply> {
  decode(payload: Uint8Array): T;
  encode(reply: Reply): Uint8Array;
}

/** What a dispatch returns. */
export interface Dispatched {
  /** The first value, in registration order, that a listener returned other than `undefined`. */
  reply: unknown;
  /** The names of the events that have listeners. */
  listened: string[];
}

const json: EventCodec<unknown, unknown> = {
  decode: decodeJson,
  encode: (reply) => reply === undefined ? new Uint8Array() : encodeJson(reply),
};

/** Every listener, by event name, in registration order. */
const listeners = new Map<string, RegisteredListener[]>();
const codecs = new Map<string, EventCodec<unknown, unknown>>();

export function listenedEvents(): string[] {
  return [...listeners.keys()];
}

export function register(name: string, listener: RegisteredListener): void {
  const registered = listeners.get(name);
  if (registered) registered.push(listener);
  else listeners.set(name, [listener]);
}

export function setCodec(name: string, codec: EventCodec<unknown, unknown>): void {
  codecs.set(name, codec);
}

/** The codec of the event `name`: the one its definition gave, or JSON. */
export function eventCodec(name: string): EventCodec<unknown, unknown> {
  return codecs.get(name) ?? json;
}

/**
 * Runs the listeners of the event `name` and waits for them to settle. A listener that throws is
 * reported with `reportError`, and the others still run.
 */
export async function dispatch(
  name: string,
  event: unknown,
  env: unknown,
  ctx: unknown,
): Promise<Dispatched> {
  const results = await Promise.allSettled(
    (listeners.get(name) ?? []).map(async (listener) => await listener(event, env, ctx)),
  );
  let reply: unknown;
  for (const result of results) {
    if (result.status === "rejected") reportError(result.reason);
    else if (reply === undefined) reply = result.value;
  }
  return { reply, listened: listenedEvents() };
}
