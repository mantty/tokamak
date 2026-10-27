import { defineEvent } from "@tokamakdev/plugin/events";

/** Runs once per runtime start, including when the system starts the app in the background. */
export const onStart = defineEvent("start");

/** Runs each time the app moves into the foreground, but not when it starts there. */
export const onResume = defineEvent("resume");

/** Runs each time the app moves out of the foreground. */
export const onSuspend = defineEvent("suspend");

export { lifecycleStage as getLifecycleStage, type LifecycleStage } from "@tokamakdev/plugin";
