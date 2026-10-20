import { register, setCodec, type EventCodec, type RegisteredListener } from "./registry.js";
import type { TokamakContext } from "./context.js";

export type { EventCodec } from "./registry.js";

declare global {
  // eslint-disable-next-line @typescript-eslint/no-namespace -- plugins add their events by declaration merging.
  namespace Tokamak {
    /** Each event's data, by event type. A plugin adds its events as `"<plugin>.<name>"`. */
    interface Events {
      /** The runtime started, including when the system started the app in the background. */
      start: { readonly foreground: boolean };
      /** The app moved into the foreground, other than when it started there. */
      resume: Readonly<Record<string, never>>;
      /** The app moved out of the foreground. */
      suspend: Readonly<Record<string, never>>;
    }
    /** The reply of each event that takes one, by event type. */
    // eslint-disable-next-line @typescript-eslint/no-empty-object-type -- plugins declare the members.
    interface EventReplies {}
  }
}

/** The type of a lifecycle or plugin event. */
export type EventType = keyof Tokamak.Events;

/** What listeners of the event `Type` may reply. */
// eslint-disable-next-line @typescript-eslint/no-invalid-void-type -- listeners of events without replies return void.
export type EventReply<Type extends EventType> = Type extends keyof Tokamak.EventReplies ? Tokamak.EventReplies[Type] : void;

/** One lifecycle or plugin event delivered by the native host. */
export type TokamakEvent = { [Type in EventType]: { readonly type: Type; readonly data: Tokamak.Events[Type] } }[EventType];

/** The invocation a listener runs in. */
export type EventContext = TokamakContext;

/** Receives an event's data with the Worker's `env`, and may return a `Reply`. */
export type Listener<T, Reply = void> = (
  event: T,
  env: Tokamak.Env,
  ctx: EventContext,
) => Reply | undefined | Promise<Reply | undefined>;

/**
 * Defines the event `type`, whose data and reply travel as JSON unless `codec` encodes them, and
 * returns the function that registers its listeners. The event's reply is the first value, in
 * registration order, that a listener returns other than `undefined`.
 */
export function defineEvent<Type extends EventType>(
  type: Type,
  codec?: EventCodec<Tokamak.Events[Type], EventReply<Type>>,
): (listener: Listener<Tokamak.Events[Type], EventReply<Type>>) => void {
  if (codec) setCodec(type, codec);
  return (listener) => {
    register(type, listener as RegisteredListener);
  };
}
