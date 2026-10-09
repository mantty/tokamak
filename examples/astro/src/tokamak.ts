import { location } from "@tokamakdev/plugin-location";
import { getLifecycleStage, onResume, onStart, onSuspend } from "@tokamakdev/tok/events";

/** Records an event in the EVENTS store, which the Events page lists. */
function record(env: Cloudflare.Env, entry: string): Promise<void> {
  return env.EVENTS.put(`${new Date().toISOString()} ${entry}`, "");
}

// Started on screen, records where location first places the device. The
// listener keeps this Worker invocation running until it removes itself.
onStart(async (event, env, ctx) => {
  ctx.waitUntil(record(env, event.foreground ? "start in the foreground" : "start in the background"));
  if ((await getLifecycleStage()) !== "foreground") return;
  const stop = location.watchPosition(
    ({ coords }) => {
      stop();
      ctx.waitUntil(record(env, `located at ${coords.latitude.toFixed(3)}, ${coords.longitude.toFixed(3)}`));
    },
    (error) => {
      stop();
      ctx.waitUntil(record(env, `location failed: ${error.name}`));
    },
  );
});
onResume((_event, env, ctx) => {
  ctx.waitUntil(record(env, "resume"));
});
onSuspend((_event, env, ctx) => {
  ctx.waitUntil(record(env, "suspend"));
});
