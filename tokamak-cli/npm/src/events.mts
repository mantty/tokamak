import { lifecycleStage, type LifecycleStage, type TokamakContext } from "@tokamakdev/plugin";
import { defineEvent } from "@tokamakdev/plugin/events";

/** Runs once per runtime start, including when the system starts the app in the background. */
export const onStart = defineEvent("start");

/** Runs each time the app moves into the foreground, but not when it starts there. */
export const onResume = defineEvent("resume");

/** Runs each time the app moves out of the foreground. */
export const onSuspend = defineEvent("suspend");

export type { LifecycleStage } from "@tokamakdev/plugin";

/**
 * The app's stage now, which can differ from the last event delivered: an `onSuspend` listener
 * that runs late reads `"foreground"` once the user has returned. Rejects with
 * `NotSupportedError` outside a tokamak Worker.
 */
export function getLifecycleStage(ctx: TokamakContext): Promise<LifecycleStage> {
  return lifecycleStage(ctx);
}
