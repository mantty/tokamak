import { nativeConnection, type TokamakContext } from "./context.js";
import { decodeJson, encodeJson } from "./codec.js";

export type { TokamakContext } from "./worker.js";
export type { CallOptions, NativePlatform, Overflow } from "./context.js";
export { capabilitiesOf, platform } from "./context.js";
export type { JsonStream } from "./streams.js";
export type { JsonValue } from "./codec.js";
export { decodeJson, encodeJson } from "./codec.js";
export { NativeReference } from "./resources.js";
export { NativeEndpoint, platformApi, type PlatformApis } from "./endpoint.js";

/** Whether the app is on screen. */
export type LifecycleStage = "foreground" | "background";

/**
 * The app's stage now, which can differ from the last lifecycle event delivered: an `onSuspend`
 * listener that runs late reads `"foreground"` once the user has returned. Rejects with
 * `NotSupportedError` without a native host.
 */
export async function lifecycleStage(ctx: TokamakContext): Promise<LifecycleStage> {
  return decodeJson<LifecycleStage>(await ctx[nativeConnection].call("tokamak", "lifecycleStage", encodeJson(null)));
}
