import { WorkerEntrypoint } from "cloudflare:workers";

import { dispatch, listenedEvents } from "./registry.ts";
import { createNativeContext, TokamakEntrypoint } from "./worker.ts";

/** The host's event adapter for the application's native entrypoint. */
export function eventEntrypoint(Worker) {
  return class extends WorkerEntrypoint {
    async dispatch(name, data) {
      const ctx = createNativeContext(this.ctx);
      const worker = new Worker(ctx, this.env);
      // Worker classes without an event handler use registered listeners.
      if (typeof worker.event !== "function") return dispatch(name, data, this.env, ctx);
      const reply = await worker.event({ type: name, data });
      const listened = worker.event === TokamakEntrypoint.prototype.event ? listenedEvents() : undefined;
      return { reply, listened };
    }
  };
}
