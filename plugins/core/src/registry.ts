/** A registered listener. */
export type RegisteredListener = (event: unknown, env: unknown, ctx: unknown) => unknown;

/** What a dispatch returns. */
export interface Dispatched {
  /** The first value, in registration order, that a listener returned other than `undefined`. */
  reply: unknown;
  /** The names of the events that have listeners. */
  listened: string[];
}

/** Every listener, by event name, in registration order. */
const listeners = new Map<string, RegisteredListener[]>();

export function register(name: string, listener: RegisteredListener): void {
  const registered = listeners.get(name);
  if (registered) registered.push(listener);
  else listeners.set(name, [listener]);
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
  return { reply, listened: [...listeners.keys()] };
}
