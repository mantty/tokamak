import { WorkerEntrypoint } from "cloudflare:workers";

import type { TokamakContext } from "./context.ts";
import { dispatch, eventCodec, listenedEvents } from "./registry.ts";
import { TokamakEntrypoint } from "./worker.ts";

/** The application's native Worker class: `TokamakEntrypoint`, or a class with an `event` handler or none. */
type NativeWorker = new (ctx: TokamakContext, env: Record<string, unknown>) => {
  event?(event: { readonly type: string; readonly data: unknown }): unknown;
};

/**
 * The host's event adapter for the application's native entrypoint. Event data and replies are
 * bytes until this endpoint decodes and encodes them with the event's codec.
 */
export function eventEntrypoint(Worker: NativeWorker, nativeContext: (ctx: ExecutionContext) => TokamakContext) {
  return class extends WorkerEntrypoint<Record<string, unknown>> {
    async dispatch(type: string, payload: Uint8Array) {
      const ctx = nativeContext(this.ctx);
      const worker = new Worker(ctx, this.env);
      const codec = eventCodec(type);
      const data = codec.decode(payload);
      // Worker classes without an event handler use registered listeners.
      const listening = typeof worker.event !== "function" || worker.event === TokamakEntrypoint.prototype.event;
      const reply = typeof worker.event === "function"
        ? await worker.event({ type, data })
        : (await dispatch(type, data, this.env, ctx)).reply;
      return { reply: codec.encode(reply), listened: listening ? listenedEvents() : undefined };
    }
  };
}
