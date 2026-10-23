import { capabilitiesFor } from "./capabilities.mjs";
import { nativeAvailability, type TokamakContext } from "./context.js";
import type { TokamakEvent } from "./events.js";
import { dispatch } from "./registry.js";

export { capabilitiesFor } from "./capabilities.mjs";
export { createNativeContext, nativeAvailability, nativeConnection, platform } from "./context.js";
export type { Availability, NativePlatform } from "./capabilities.mjs";
export type { TokamakContext } from "./context.js";
export type { WebView } from "./webview.js";
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

/** The page module that tells generated clients which plugin operations the app's platform implements. */
export function capabilitiesModule(ctx: TokamakContext): Response {
  const capabilities = capabilitiesFor(ctx[nativeAvailability], ctx.platform);
  return new Response(`export default ${JSON.stringify(capabilities)};\n`, {
    headers: { "content-type": "text/javascript; charset=utf-8", "cache-control": "no-store" },
  });
}
