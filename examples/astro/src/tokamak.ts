import { location } from "@tokamakdev/plugin-location";
import { notifications } from "@tokamakdev/plugin-notifications";
import {
  onNotificationDismissed,
  onNotificationOpened,
  onPush,
} from "@tokamakdev/plugin-notifications/events";
import { getLifecycleStage, onResume, onStart, onSuspend } from "@tokamakdev/tok/events";

/** Records an event in the EVENTS store, which the Events page lists. */
function record(env: Cloudflare.Env, entry: string): Promise<void> {
  return env.EVENTS.put(`${new Date().toISOString()} ${entry}`, "");
}

// Started on screen, records where location first places the device. The
// listener keeps this Worker invocation running until it removes itself.
onStart(async (event, env, ctx) => {
  ctx.waitUntil(record(env, event.foreground ? "start in the foreground" : "start in the background"));
  ctx.waitUntil(setUpNotifications(env));
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

// Records each data-only push message, and shows a notification of its data.
onPush(async (message, env) => {
  await record(env, `push message ${message.id}`);
  return { id: `push-${message.id}`, title: "Push message", body: JSON.stringify(message.data) };
});
onNotificationOpened((opened, env, ctx) => {
  const action = opened.action === null ? "" : ` with ${opened.action}`;
  const input = opened.input === null ? "" : `: ${opened.input}`;
  ctx.waitUntil(record(env, `opened ${opened.source} notification ${opened.id}${action}${input}`));
});
onNotificationDismissed((dismissed, env, ctx) => {
  ctx.waitUntil(record(env, `dismissed ${dismissed.source} notification ${dismissed.id}`));
});

/**
 * Configures notifications on every platform at each start, recording channel settings Android
 * kept. Pass text in the user's language: the system shows these names and titles.
 */
async function setUpNotifications(env: Cloudflare.Env): Promise<void> {
  const { unapplied } = await notifications.setup({
    categories: [
      {
        id: "message",
        actions: [
          { id: "reply", title: "Reply", input: { button: "Send", placeholder: "Message" } },
          // Wearables and the web show the icon.
          { id: "read", title: "Mark as read", icon: "/notifications/icon.png" },
          { id: "open", title: "Open", foreground: true },
        ],
        hiddenPreviewsBodyPlaceholder: "New message",
        customDismissAction: true,
      },
    ],
    showInForeground: true,
    defaultChannel: { name: "General", description: "Notifications from the example" },
    channelGroups: [{ id: "chats", name: "Chats" }],
    channels: [
      {
        id: "messages",
        name: "Messages",
        description: "New messages",
        group: "chats",
        importance: "high",
      },
    ],
  });
  for (const { channel, fields } of unapplied) {
    await record(env, `channel ${channel} kept its ${fields.join(", ")}`);
  }
}
