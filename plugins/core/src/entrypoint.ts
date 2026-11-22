import { WorkerEntrypoint } from "cloudflare:workers";

import type { TokamakContext } from "./context.ts";
import { eventCodec } from "./events.ts";

/** The application's native Worker class, whose handlers receive the host's events. */
type NativeWorker = new (ctx: TokamakContext, env: Record<string, unknown>) => object;

/** The host's event adapter for the application's native entrypoint. */
export function eventEntrypoint(Worker: NativeWorker, nativeContext: (ctx: ExecutionContext) => TokamakContext) {
  return class extends WorkerEntrypoint<Record<string, unknown>> {
    /**
     * Calls the Worker's `handler` with the event's decoded `payload` and resolves to its encoded
     * reply, which is empty when the Worker has no such handler or it returns nothing. A lifecycle
     * handler is the Worker's member of that name; a plugin's, `<plugin id>.<event>`, is the
     * function `<event>` in the Worker's `_<plugin id>` property. What the handler throws is
     * reported with `reportError`.
     */
    async dispatch(handler: string, payload: Uint8Array): Promise<Uint8Array> {
      const worker = new Worker(nativeContext(this.ctx), this.env);
      const dot = handler.indexOf(".");
      const holder: unknown = dot < 0 ? worker : Reflect.get(worker, `_${handler.slice(0, dot)}`);
      const method: unknown = typeof holder === "object" && holder !== null ? Reflect.get(holder, handler.slice(dot + 1)) : undefined;
      if (typeof method !== "function") return new Uint8Array();
      const codec = eventCodec(handler);
      try {
        const reply: unknown = await Reflect.apply(method, holder, [codec.decode(payload)]);
        return reply === undefined ? new Uint8Array() : codec.encode(reply);
      } catch (error) {
        reportError(error);
        return new Uint8Array();
      }
    }
  };
}
