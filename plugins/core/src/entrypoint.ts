import { WorkerEntrypoint } from "cloudflare:workers";

import type { TokamakContext } from "./context.ts";
import { eventCodec } from "./events.ts";

/** The application's native Worker class, whose methods handle the host's events. */
type NativeWorker = new (ctx: TokamakContext, env: Record<string, unknown>) => object;

/** The host's event adapter for the application's native entrypoint. */
export function eventEntrypoint(Worker: NativeWorker, nativeContext: (ctx: ExecutionContext) => TokamakContext) {
  return class extends WorkerEntrypoint<Record<string, unknown>> {
    /**
     * Calls the Worker's method `handler` with the event's decoded `payload` and resolves to its
     * encoded reply, which is empty when the Worker has no such method or it returns nothing. What
     * the method throws is reported with `reportError`.
     */
    async dispatch(handler: string, payload: Uint8Array): Promise<Uint8Array> {
      const worker = new Worker(nativeContext(this.ctx), this.env);
      const method: unknown = Reflect.get(worker, handler);
      if (typeof method !== "function") return new Uint8Array();
      const codec = eventCodec(handler);
      try {
        const reply: unknown = await Reflect.apply(method, worker, [codec.decode(payload)]);
        return reply === undefined ? new Uint8Array() : codec.encode(reply);
      } catch (error) {
        reportError(error);
        return new Uint8Array();
      }
    }
  };
}
