import { capabilitiesFor } from "./capabilities.js";
import { nativeAvailability, type TokamakContext } from "./context.js";

export { capabilitiesFor } from "./capabilities.js";
export { createNativeContext, nativeAvailability, nativeConnection, platform } from "./context.js";
export type { Availability, NativePlatform } from "./capabilities.js";
export type { TokamakContext } from "./context.js";
export type { WebView } from "./webview.js";
export type { StartEvent } from "./events.js";

/** `Object`, typed with the handlers a Tokamak worker's class may define, as `Tokamak.Handlers` declares them. */
const Handlers: new () => Tokamak.Handlers = Object;

/**
 * The application's native worker entrypoint. The native host calls its `Tokamak.Handlers`
 * methods, and pages and the app's Worker call its other methods whose names do not start with `_`.
 */
export class TokamakEntrypoint<Env = Tokamak.Env> extends Handlers {
  constructor(
    protected readonly ctx: TokamakContext,
    protected readonly env: Env,
  ) {
    super();
  }
}

/** The page module that tells generated clients which plugin operations the app's platform implements. */
export function capabilitiesModule(ctx: TokamakContext): Response {
  const capabilities = capabilitiesFor(ctx[nativeAvailability], ctx.platform);
  return new Response(`export default ${JSON.stringify(capabilities)};\n`, {
    headers: { "content-type": "text/javascript; charset=utf-8", "cache-control": "no-store" },
  });
}
