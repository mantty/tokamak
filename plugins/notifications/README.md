# @tokamakdev/plugin-notifications

Local and push notifications for tokamak applications and the web.

```sh
npm install @tokamakdev/plugin-notifications
```

```ts
import { notifications } from "@tokamakdev/plugin-notifications";

await notifications.requestPermission(); // "granted" | "denied" | "prompt"

await notifications.show({ id: "download-42", title: "Download finished", body: "report.pdf" });
await notifications.schedule({ id: "reminder-7", title: "Check in", at: Date.parse("2026-10-01T09:00:00Z") });

const subscription = await notifications.subscribe({ applicationServerKey: VAPID_PUBLIC_KEY });
await fetch("/api/subscriptions", { method: "POST", body: JSON.stringify(subscription) });

notifications.onMessage((message) => console.log(message.data));
notifications.onNotificationOpened((opened) => console.log(opened.id, opened.source));
notifications.onSubscriptionChange((subscription) => {
  void fetch("/api/subscriptions", { method: "POST", body: JSON.stringify(subscription) });
});
```

## Permission

`permission()` reports whether the app may show notifications and
`requestPermission()` asks the user. No other method asks. On Android before
13, notifications are allowed unless the user turns them off.

While the app is off screen on iOS or Android, as when Worker code runs in the
background, a `requestPermission()` that would show the prompt rejects with
`NeedsUIError`; one the user has already answered resolves with the
permission. Ask again once the app is on screen, for example from `onResume`.

## Local notifications

- `show(notification)` shows a notification now, even while the page is
  visible.
- `schedule(notification)` shows it at `at`, in milliseconds since the epoch,
  or now when `at` has passed. Scheduled notifications survive app and device
  restarts. Android may delay them by several minutes while the device is idle.
  iOS and macOS keep at most 64; scheduling more rejects with
  `QuotaExceededError`. The web does not support scheduling.
- `getScheduled()` lists scheduled notifications not yet shown, and
  `getDelivered()` lists shown notifications still in the notification centre.
- `remove(id)` removes a scheduled or shown notification. On Android, the id
  of a push notification FCM showed is its `tag`.

Showing or scheduling a notification with the `id` of another replaces it.

## Push notifications

`subscribe()` subscribes to push messages and returns where the app's server
sends them. It returns the existing subscription when there is one. In native
builds it shows no UI, so the Worker can subscribe at every start, whether or
not the app starts on screen:

```ts
// src/tokamak.ts
import { onStart } from "@tokamakdev/tok/events";
import { notifications } from "@tokamakdev/plugin-notifications";

onStart(async () => {
  const subscription = await notifications.subscribe();
  await fetch("https://api.example.com/push-tokens", { method: "POST", body: JSON.stringify(subscription) });
});
```

| Platform | Subscription | Push service |
|---|---|---|
| iOS, macOS | `{ service: "apns", token, environment }` | APNs, in `environment` (`development` or `production`) |
| Android | `{ service: "fcm", token }` | Firebase Cloud Messaging HTTP v1 |
| Web | `{ service: "webpush", endpoint, keys }` | Web Push, with the app's VAPID key pair |

Push notifications that arrive while the page is visible reach `onMessage`.
They are also shown when `subscribe` was given `showInForeground: true`, on
iOS, macOS and Android. The web shows every push notification.
`onSubscriptionChange` reports a replacement subscription, for example when FCM
rotates a token.

### Declarations

Local notifications need no declarations. Push needs these:

| Platform | Declaration |
|---|---|
| iOS | `aps-environment` in the app's entitlements file, set with `ios.entitlements`. The value may be `development`: signing uses the provisioning profile's value. Push requires a paid Apple Developer Program team. |
| macOS | `com.apple.developer.aps-environment` in the `macos.entitlements` file, and a team-signed build (`macos.team-id`). |
| Android | The app's Firebase project values as `<meta-data>` in its `android.manifest` file, from the Firebase console's project settings. |
| Web | A service worker that uses the helper below, and the app's VAPID public key as `applicationServerKey`. |

```xml
<!-- iOS entitlements file -->
<plist version="1.0">
<dict>
  <key>aps-environment</key>
  <string>development</string>
</dict>
</plist>
```

```xml
<!-- Android manifest file -->
<manifest xmlns:android="http://schemas.android.com/apk/res/android">
  <application>
    <meta-data android:name="com.tokamak.notifications.fcm.application-id" android:value="1:1234567890:android:0123456789abcdef" />
    <meta-data android:name="com.tokamak.notifications.fcm.project-id" android:value="my-project" />
    <meta-data android:name="com.tokamak.notifications.fcm.api-key" android:value="AIza..." />
  </application>
</manifest>
```

Without its declaration, `subscribe` rejects with `InvalidStateError`, and in
a macOS build without a team with `NotSupportedError`. The Firebase values
identify the project and are not secret.

The iOS build declares the `remote-notification` background mode. An
`ios.plist` file that sets `UIBackgroundModes` replaces the plugin's value, so
it must include `remote-notification`.

### Messages

The server sends each service's payload, and the plugin reports it as a
`Message`:

| Service | `id` | `title`, `body` | `data` |
|---|---|---|---|
| APNs | The notification's identifier | `aps.alert.title`, `aps.alert.body` | Every top-level key except `aps` |
| FCM | The message ID | `notification.title`, `notification.body` | The `data` map |
| Web Push | `id` in the JSON payload | `title`, `body` in the payload | `data` in the payload |

A Web Push payload is JSON: `{ "id": "...", "title": "...", "body": "...", "data": {} }`.
Every field is optional.

### Data-only messages

In native builds, the Worker's `onPush` listeners receive each message without
a title or body, whether the app is in the foreground, in the background, or
started by the message. Register them in `src/tokamak.ts`, the module the
tokamak Vite plugin runs only on the device:

```ts
// src/tokamak.ts
import { onPush } from "@tokamakdev/plugin-notifications/events";

onPush(async (message) => {
  const item = await fetch(`https://api.example.com/items/${String(message.data.itemId)}`);
  // Return a notification to show it, or return nothing.
  return { id: `item-${String(message.data.itemId)}`, title: "New item", body: (await item.json()).summary };
});
```

`message` is the `Message`, whose `data` is `Record<string, unknown>`; FCM
delivers every `data` value as a string. The first notification a listener
returns is shown as `show` shows one. The page also receives the message
through `onMessage` while it is loaded. Builds for Cloudflare and the web
include no listeners.

The listeners have until the plugin's time for the message runs out, which
includes starting the app. The plugin logs a message it could not deliver to
the device log as `push notification failed: <reason>`.

The platforms limit background delivery:

| Platform | The server sends | Limits |
|---|---|---|
| iOS | `content-available: 1` with `apns-push-type: background` and `apns-priority: 5` | The system throttles delivery and may drop messages, and delivers nothing after the user force-quits the app until they open it. The plugin's time for a message is 25 seconds, within the system's 30. |
| macOS | As iOS | Delivered while the app is running. |
| Android | A `data` message with `priority: high` | Nothing is delivered after the app is force-stopped. The plugin's time for a message is 8 seconds, within FCM's 10, which include starting the app. |

The web does not support data-only messages: browsers require every push
message to show a notification.

## Opened notifications

`onNotificationOpened` receives each notification the user opens, local or
push. The plugin holds notifications opened before a page listens, including
the one that launched the app, and delivers them to the first listener.
Android does not pass the title or body of an FCM notification the system
showed, so those are null.

In native builds, the Worker's `onNotificationOpened` listeners, from
`@tokamakdev/plugin-notifications/events`, also receive each opened
notification, with the same `OpenedNotification`:

```ts
// src/tokamak.ts
import { onNotificationOpened } from "@tokamakdev/plugin-notifications/events";

onNotificationOpened((opened, env, ctx) => {
  ctx.waitUntil(env.OPENS.put(opened.id, opened.source));
});
```

## Web

The web implementation needs a service worker for everything but `show`. Add
the helpers to the app's own service worker:

```js
import {
  handleNotificationClick,
  handlePush,
  handleSubscriptionChange,
} from "@tokamakdev/plugin-notifications/service-worker";

self.addEventListener("push", (event) => handlePush(event));
self.addEventListener("notificationclick", (event) => handleNotificationClick(event));
self.addEventListener("pushsubscriptionchange", (event) => handleSubscriptionChange(event));
```

- `handlePush` shows the payload's notification and forwards the message to
  open pages.
- `handleNotificationClick` focuses an open page, or opens `data.url` (default
  `/`), and holds the notification for `onNotificationOpened`. It returns false
  for notifications the plugin did not show.
- `handleSubscriptionChange` forwards a replacement subscription to open pages.

Safari supports Web Push on macOS 13 and later, and on iOS and iPadOS 16.4 and
later only for web apps added to the Home Screen.

## Android icon

Android shows notifications with a monochrome icon. Add one as
`drawable/tokamak_notification` in the app's Android resource directory, set
with `android.icon`. Without it, the plugin uses the launcher icon, which
Android draws as a solid shape.

## Errors

- `NotAllowedError`: permission has not been granted.
- `NeedsUIError`: `requestPermission()` would show the prompt while the app is
  off screen on iOS or Android.
- `NotSupportedError`: the platform or build cannot perform the call.
- `InvalidStateError`: the app has not made a declaration push requires.
- `QuotaExceededError`: the platform's limit on scheduled notifications is
  reached.
- `OperationError`: a platform or push service failure.
- `TypeError`: an invalid argument.
