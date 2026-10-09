import { Plugin } from "@tokamakdev/plugin";
import { defineEvent } from "@tokamakdev/plugin/events";

/** An event that carries no data. */
type Empty = Readonly<Record<string, never>>;

/** Runs once per runtime start, including when the system starts the app in the background. */
export const onStart = defineEvent<{ readonly foreground: boolean }>("start");

/** Runs each time the app moves into the foreground, but not when it starts there. */
export const onResume = defineEvent<Empty>("resume");

/** Runs each time the app moves out of the foreground. */
export const onSuspend = defineEvent<Empty>("suspend");

/** Whether the app is on screen. */
export type LifecycleStage = "foreground" | "background";

/** The functions the runtime answers itself, as the reserved plugin `tokamak`. */
class Tokamak extends Plugin {
  constructor() {
    super("tokamak");
  }

  lifecycleStage(): Promise<LifecycleStage> {
    return this.call("lifecycleStage");
  }
}

const tokamak = new Tokamak();

/**
 * The app's stage now, which can differ from the last event delivered: an `onSuspend` listener
 * that runs late reads `"foreground"` once the user has returned. Rejects with
 * `NotSupportedError` outside a tokamak Worker.
 */
export function getLifecycleStage(): Promise<LifecycleStage> {
  return tokamak.lifecycleStage();
}
