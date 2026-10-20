import type { TokamakContext } from "./context.js";
import type { TokamakEvent } from "./events.js";
import { dispatch } from "./registry.js";

export { createNativeContext, nativeConnection } from "./context.js";
export type { TokamakContext } from "./context.js";
export type { EventReply, EventType, TokamakEvent } from "./events.js";

/** The application's native worker entrypoint. */
export class TokamakEntrypoint<Env = Tokamak.Env> {
  constructor(
    protected readonly ctx: TokamakContext,
    protected readonly env: Env,
  ) {}

  /** Runs the listeners registered for this event and resolves to their reply. */
  event(event: TokamakEvent): unknown {
    return dispatch(event.type, event.data, this.env, this.ctx).then(({ reply }) => reply);
  }
}
