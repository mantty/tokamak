import type { TokamakContext } from "./context.js";
import { dispatch } from "./registry.js";

export { createNativeContext, nativeConnection } from "./context.js";
export type { TokamakContext } from "./context.js";

/** One lifecycle or plugin event delivered by the native host. */
export interface TokamakEvent<T = unknown> {
  readonly type: string;
  readonly data: T;
}

/** The application's native worker entrypoint. */
export class TokamakEntrypoint<Env = Tokamak.Env> {
  constructor(
    protected readonly ctx: TokamakContext,
    protected readonly env: Env,
  ) {}

  /** Runs the listeners registered for this event. */
  event(event: TokamakEvent): unknown {
    return dispatch(event.type, event.data, this.env, this.ctx).then(({ reply }) => reply);
  }
}
