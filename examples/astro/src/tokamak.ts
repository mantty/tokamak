import {
  TokamakEntrypoint,
  getLifecycleStage,
  type StartEvent,
  type TokamakContext,
} from "@tokamakdev/tok";
import { location, type LocationHandlers } from "@tokamakdev/plugin-location";
import {
  notifications,
  type NotificationsHandlers,
} from "@tokamakdev/plugin-notifications";

export default class Native extends TokamakEntrypoint {
  lifecycleStage() {
    return getLifecycleStage(this.ctx);
  }

  /** Records an event in the EVENTS store, which the Events page lists. */
  _record(entry: string): Promise<void> {
    return this.env.EVENTS.put(`${new Date().toISOString()} ${entry}`, "");
  }

  // Inside the platform's launch, sets up notifications and long-term location awareness:
  // significant changes everywhere, and visits on Apple platforms.
  _beforeStart = async () => {
    await this._record("before start");
    for (const { channel, fields } of await setUpNotifications(this.ctx)) {
      await this._record(`channel ${channel} kept its ${fields.join(", ")}`);
    }
    const awareness = location(this.ctx);
    try {
      await awareness.startSignificantChanges();
      if (awareness.capabilities.startVisits) await awareness.startVisits();
    } catch (error) {
      await this._record(`location awareness failed: ${String(error)}`);
    }
  };

  // Records each start and, on screen once the user has answered the location prompt, where
  // location first places the device.
  _onStart = async ({ foreground }: StartEvent) => {
    await this._record(
      foreground ? "start in the foreground" : "start in the background",
    );
    if (!foreground) return;
    // `_onStart` has 10 seconds and every event waits for it, so it never waits for the user to
    // answer a prompt, and the lookup leaves time to record its outcome.
    if ((await location(this.ctx).permission()).inUse === "prompt") return;
    try {
      const { coords } = await location(this.ctx).getCurrentPosition({
        maximumAge: 60_000,
        timeout: 5_000,
      });
      await this._record(
        `located at ${coords.latitude.toFixed(3)}, ${coords.longitude.toFixed(3)}`,
      );
    } catch (error) {
      await this._record(`location failed: ${String(error)}`);
    }
  };

  _onResume = () => this._record("resume");

  _onSuspend = () => this._record("suspend");

  // The updates, region transitions, visits and failures that reach the Worker, including those
  // the system launches the app for.
  _location: LocationHandlers = {
    onUpdate: ({ positions, stationary }) => {
      const last = positions.at(-1)?.coords;
      const at = last
        ? ` at ${last.latitude.toFixed(3)}, ${last.longitude.toFixed(3)}`
        : "";
      return this._record(
        `location update of ${String(positions.length)}${at}${stationary ? ", stationary" : ""}`,
      );
    },
    onRegionTransition: ({ id, transition }) =>
      this._record(`region ${id}: ${transition}`),
    onVisit: ({ latitude, longitude, departure }) =>
      this._record(
        `visit at ${latitude.toFixed(3)}, ${longitude.toFixed(3)}${departure === null ? "" : ", departed"}`,
      ),
    onError: ({ name, message, id }) =>
      this._record(
        `location failure${id === undefined ? "" : ` of ${id}`}: ${name}: ${message}`,
      ),
  };

  _notifications: NotificationsHandlers = {
    // Records each data-only push message, and shows a notification of its data.
    onPush: async (message) => {
      await this._record(`push message ${message.id}`);
      return {
        id: `push-${message.id}`,
        title: "Push message",
        body: JSON.stringify(message.data),
      };
    },
    onOpened: (opened) => {
      const action = opened.action === null ? "" : ` with ${opened.action}`;
      const input = opened.input === null ? "" : `: ${opened.input}`;
      return this._record(
        `opened ${opened.source} notification ${opened.id}${action}${input}`,
      );
    },
    onDismissed: (dismissed) =>
      this._record(`dismissed ${dismissed.source} notification ${dismissed.id}`),
  };
}

/**
 * Configures notifications on every platform, returning the channel settings Android kept. Pass
 * text in the user's language: the system shows these names and titles.
 */
async function setUpNotifications(ctx: TokamakContext) {
  const { unapplied } = await notifications(ctx).setup({
    categories: [
      {
        id: "message",
        actions: [
          {
            id: "reply",
            title: "Reply",
            input: { button: "Send", placeholder: "Message" },
          },
          // Wearables and the web show the icon.
          {
            id: "read",
            title: "Mark as read",
            icon: "/notifications/icon.png",
          },
          { id: "open", title: "Open", foreground: true },
        ],
        hiddenPreviewsBodyPlaceholder: "New message",
        customDismissAction: true,
      },
    ],
    showInForeground: true,
    defaultChannel: {
      name: "General",
      description: "Notifications from the example",
    },
    channelGroups: [{ id: "chats", name: "Chats" }],
    channels: [
      {
        id: "messages",
        name: "Messages",
        description: "New messages",
        group: "chats",
        importance: "high",
      },
      {
        id: "chimes",
        name: "Chimes",
        description: "Notifications with a chime",
        importance: "high",
        sound: "chime.wav",
      },
    ],
  });
  return unapplied;
}
