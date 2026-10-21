import { WorkerEntrypoint } from "cloudflare:workers";

import { dispatch, eventCodec, listenedEvents } from "./registry.ts";
import { TokamakEntrypoint } from "./worker.ts";

/**
 * The host's event adapter for the application's native entrypoint. Event data and replies are
 * bytes until this endpoint decodes and encodes them with the event's codec.
 */
export function eventEntrypoint(Worker, nativeContext) {
  return class extends WorkerEntrypoint {
    async dispatch(type, payload) {
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
