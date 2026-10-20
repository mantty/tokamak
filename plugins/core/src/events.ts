import { register, type RegisteredListener } from "./registry.js";
import type { TokamakContext } from "./context.js";

/** The invocation a listener runs in. */
export type EventContext = TokamakContext;

/** Receives an event of type `T` with the Worker's `env`, and may return a `Reply`. */
export type Listener<T, Reply = void> = (
  event: T,
  env: Cloudflare.Env,
  ctx: EventContext,
) => Reply | undefined | Promise<Reply | undefined>;

/**
 * Defines the event `name` and returns the function that registers its listeners. The event's
 * reply is the first value, in registration order, that a listener returns other than
 * `undefined`.
 */
export function defineEvent<T, Reply = void>(
  name: string,
): (listener: Listener<T, Reply>) => void {
  return (listener) => {
    register(name, listener as RegisteredListener);
  };
}
