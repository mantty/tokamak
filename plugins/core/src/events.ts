import { register, type RegisteredListener } from "./registry.js";

declare global {
  // eslint-disable-next-line @typescript-eslint/no-namespace -- `wrangler types` declares the Worker's bindings here.
  namespace Cloudflare {
    // eslint-disable-next-line @typescript-eslint/no-empty-object-type -- the app's `wrangler types` declares the members.
    interface Env {}
  }
}

/** The invocation a listener runs in. */
export interface EventContext {
  /** Keeps the event running until `promise` settles or the event's deadline passes. */
  waitUntil(promise: Promise<unknown>): void;
}

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
