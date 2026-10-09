import { onResume, onStart, onSuspend } from "@tokamakdev/tok/events";

/** Records an event in the EVENTS store, which the Events page lists. */
function record(env: Cloudflare.Env, entry: string): Promise<void> {
  return env.EVENTS.put(`${new Date().toISOString()} ${entry}`, "");
}

onStart((event, env, ctx) => {
  ctx.waitUntil(record(env, event.foreground ? "start in the foreground" : "start in the background"));
});
onResume((_event, env, ctx) => {
  ctx.waitUntil(record(env, "resume"));
});
onSuspend((_event, env, ctx) => {
  ctx.waitUntil(record(env, "suspend"));
});
