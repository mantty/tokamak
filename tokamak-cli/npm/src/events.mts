import { defineEvent } from "@tokamakdev/plugin/events";

/** An event that carries no data. */
type Empty = Readonly<Record<string, never>>;

/** Runs once per runtime start, including when the system starts the app in the background. */
export const onStart = defineEvent<{ readonly foreground: boolean }>("start");

/** Runs each time the app moves into the foreground, but not when it starts there. */
export const onResume = defineEvent<Empty>("resume");

/** Runs each time the app moves out of the foreground. */
export const onSuspend = defineEvent<Empty>("suspend");
