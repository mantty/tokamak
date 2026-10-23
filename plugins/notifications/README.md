# @tokamakdev/plugin-notifications

Local and push notifications for tokamak applications and the web.

```sh
npm install @tokamakdev/plugin-notifications
```

Pages import the generated `/client` SDK. In `src/tokamak.ts`, import the
package root and call its factory with the handler's `ctx`. The main worker
uses its generated `env.TOKAMAK_NOTIFICATIONS` service binding when present.

```ts
import notifications from "@tokamakdev/plugin-notifications/client";

await notifications.requestPermission(); // "granted" | "denied" | "prompt"

await notifications.show({ id: "download-42", title: "Download finished", body: "report.pdf" });
await notifications.schedule({ id: "reminder-7", title: "Check in", at: Date.parse("2026-10-01T09:00:00Z") });
await notifications.schedule({ id: "standup", title: "Stand-up", on: { weekday: 2, hour: 9, minute: 30 }, repeats: true });

const subscription = await notifications.subscribe({ applicationServerKey: VAPID_PUBLIC_KEY });
await fetch("/api/subscriptions", { method: "POST", body: JSON.stringify(subscription) });

import { decodeValues } from "@tokamakdev/plugin/streams";
import type { Message } from "@tokamakdev/plugin-notifications";
for await (const message of decodeValues<Message>(notifications.onMessage())) {
  console.log(message.data);
}
```

Subscription methods return byte streams of JSON values. Use `decodeValues<T>`
to read them and cancel the stream when finished. Stream errors terminate
the subscription. Background handlers are registered separately via `/events`.

## Permission

`permission()` reports whether the app may show notifications and
`requestPermission()` asks the user. No other method asks. On Android before
13, notifications are allowed unless the user turns them off.

While the app is off screen on iOS or Android, as when Worker code runs in the
background, a `requestPermission()` that would show the prompt rejects with
`NeedsUIError`; one the user has already answered resolves with the
permission. Ask again once the app is on screen, for example from `onResume`.

On Apple platforms, `requestPermission(options)` asks for what `options`
names. `alert`, `sound` and `badge` are true by default:

- `provisional`: grants permission without a prompt, and notifications go
  quietly to Notification Center until the user keeps them. A later
  `requestPermission()` without it asks for full permission.
- `criticalAlert`: critical alerts, which need the critical alerts
  entitlement.
- `providesAppNotificationSettings`: the system's notification settings link
  to the app's own, which `onOpenSettings` receives.
- `carPlay` (iOS).

An app that already has permission can call `requestPermission` again to add
`criticalAlert`, which shows its own prompt, or
`providesAppNotificationSettings`, which shows none.

### Settings

- `settings()` returns `permission` and the platform's detail.
  - **Apple:** `authorizationStatus`, `alertStyle`, `showPreviews`,
    `providesAppNotificationSettings`, and each setting (`alert`, `sound`,
    `badge`, `lockScreen`, `notificationCenter`, `criticalAlert`,
    `timeSensitive`, `scheduledDelivery`, `directMessages`, and on iOS
    `carPlay` and `announcement`) as `enabled`, `disabled` or `notSupported`.
  - **Android:** each channel's current `importance`, which the user may
    have changed (`none` when blocked), `blockedChannelGroups`,
    `doNotDisturbAccess`, `exactAlarms` and `fullScreenIntent`.
  - **Web:** `permission` only.
- `setBadge(count)` sets the app's badge, and `0` removes it. The web uses the
  Badging API, which browsers give installed web apps. Android has no API for
  a launcher's count, so it rejects with `NotSupportedError`; a
  notification's `badge` sets Android's number.
- `openSettings(options?)` opens the app's page of the system's notification
  settings, and needs the app on screen. On Android, `options` can name one
  other page: a `channel`'s, `exactAlarms`, `fullScreenIntent` (Android 14) or
  `doNotDisturbAccess`. The web rejects with `NotSupportedError`.
- `onOpenSettings` receives the user's requests, from the system's
  notification settings, for the app's own: `{ channel, notification }`, the
  channel whose settings the user was in (Android) and the id of the
  notification they came from, each absent when there is none. On Apple platforms the system offers
  the link after `requestPermission({ providesAppNotificationSettings: true })`.
  Android shows it in the app's notification settings, and in a
  notification's long-press menu as its `settingsText`. The plugin holds the
  request that opened the app until a page listens.

## Setup

`setup(config)` configures notifications on every platform in one call. Each
platform reads its own keys and ignores the others, so the same call runs
everywhere:

```ts
// src/tokamak.ts
import { onStart } from "@tokamakdev/tok/events";
import { notifications } from "@tokamakdev/plugin-notifications";

onStart((_event, _env, ctx) => {
  ctx.waitUntil(
    notifications(ctx).setup({
      categories: [
        {
          id: "message",
          actions: [
            { id: "reply", title: "Reply", input: { button: "Send", placeholder: "Message" } },
            { id: "read", title: "Mark as read" },
            { id: "open", title: "Open", foreground: true },
          ],
          customDismissAction: true,
        },
      ],
      showInForeground: { banner: true, list: true },
      defaultChannel: { name: "General" },
      channels: [{ id: "messages", name: "Messages", importance: "high" }],
    }),
  );
});
```

```ts
await notifications.show({ id: "message-1", title: "Ada", body: "Lunch?", category: "message", channel: "messages" });
```

Native builds run `src/tokamak.ts` at every start, including in the
background. Web builds leave it out, so a web app calls `setup` from page
code, for example where it registers its service worker. The page stores the
configuration where the service worker reads it.

The configuration is the whole of it, not a change to the last one:

- `setup` checks every value first. An invalid one rejects with `TypeError`
  and changes nothing.
- It removes the categories, channels and channel groups an earlier
  configuration had and this one leaves out.
- The plugin keeps the configuration and applies it again when the app starts,
  before the Worker runs, so an action that starts the app in the background
  works. Calling `setup` at every start with the same configuration changes
  nothing.

The system shows action titles, placeholders and channel names as given, so
pass them in the user's language.

### Categories and actions

A notification that names a `category` shows the category's actions. Choosing
one removes the notification and reaches `onNotificationOpened` with the
action's `id` as `action`, and the text typed into an action with `input` as
`input`. An action without `foreground` does not open the app: the Worker's
listeners receive it, starting the app in the background if it is not running,
and so do the page's while it is loaded.

| Key | iOS, macOS | Android | Web |
|---|---|---|---|
| `actions` | Every action, or the first two where space is short | Up to three | Up to `Notification.maxActions`, two in Chrome; none in Safari |
| `input` | A text field with `button` and `placeholder` | An inline reply labelled `placeholder` | A text field labelled `placeholder`, in Chrome only |
| `foreground` | Opens the app | Opens the app | Focuses a page, or opens `data.url` |
| `icon` | No | On wearables | In Chrome |
| `destructive`, `authenticationRequired`, `systemImage` | Yes | No | No |
| `hiddenPreviewsBodyPlaceholder`, `hiddenPreviewsShowTitle`, `hiddenPreviewsShowSubtitle`, `summaryFormat`, `customDismissAction` | Yes | No | No |
| `allowInCarPlay` | iOS | No | No |

Push notifications name a category in their payload: `aps.category` for APNs
and `category` for Web Push. FCM notification messages have none; to give a
push notification actions on Android, send a data-only message and return the
notification, with its `category`, from `onPush`.

### While the app is on screen

`showInForeground` sets whether notifications show while the app is on screen,
local or push: `true`, the default, or `false`. Apple platforms also take an
object naming the parts to show, from `banner`, `list`, `sound` and `badge`.
The web always shows notifications. Push messages reach `onMessage` either
way.

### Android channels

Android files each notification under a channel, whose settings the user can
change in system settings. A notification that names no `channel` uses the
default channel, which is named after the app unless `defaultChannel` names
it. `channels` adds channels, and `channelGroups` groups them in system
settings. FCM notification messages use the channel their
`android.notification.channel_id` names.

- `importance`: `none`, `min`, `low`, `default` or `high`, which shows
  heads-up notifications. Defaults to `default`.
- `sound`: `"default"`, `"none"`, or a file of the `sounds` build setting,
  such as `"chime.wav"` (below).
- `vibration`: whether to vibrate, or a non-empty pattern of milliseconds
  alternating between off and on.
- `lights`: the light's colour as `#rrggbb`, such as `"#00ff00"`, or false.
- `showBadge`: whether the launcher shows a badge.
- `bypassDnd`: whether notifications sound during Do Not Disturb. It takes
  effect only for apps the user gave Do Not Disturb access.

Android keeps an existing channel's settings as first created or as the user
changed them, and lets `setup` change only its name and description, its
group while it has none, and its importance downwards while the user has
changed nothing. `setup` resolves with the settings it could not change:

```ts
const { unapplied } = await notifications.setup(config);
// [{ channel: "messages", fields: ["importance"] }]
```

To change a channel's behaviour for existing installations, give it a new
`id`. Android restores a removed channel's settings when an app creates a
channel with its `id` again.

## Local notifications

- `show(notification)` shows a notification now. While the app is on screen,
  it shows as `showInForeground` sets.
- `schedule(notification)` shows it when its trigger fires, below. Scheduled
  notifications survive app and device restarts. iOS and macOS keep at most
  64; scheduling more rejects with `QuotaExceededError`. The web does not
  support scheduling.
- `getScheduled()` lists scheduled notifications not yet shown, and
  `getDelivered()` lists shown notifications still in the notification centre.
- `remove(id)` removes a scheduled or shown notification. On Android, the id
  of a push notification FCM showed is its `tag`.

Showing or scheduling a notification with the `id` of another replaces it.
`show` and `schedule` check every field first and reject with `TypeError`,
showing nothing, when one is invalid. On Apple platforms and Android,
`getScheduled` and `getDelivered` return the fields of a local notification
that the platform uses, without the other platforms' fields.

### Scheduling

A scheduled notification has one trigger:

```ts
await notifications.schedule({ ...content, at: Date.parse("2026-11-01T09:00:00Z") });
await notifications.schedule({ ...content, every: { hours: 1 } });
await notifications.schedule({ ...content, on: { weekday: 2, hour: 9, minute: 0 }, repeats: true });
await notifications.schedule({ ...content, region: { latitude, longitude, radius: 100, onExit: true } });
await notifications.schedule({ ...content, at, exact: true });
```

- `at`: a time in milliseconds since the epoch. A time that has passed shows
  the notification now.
- `every`: `{ seconds?, minutes?, hours?, days? }`, at least 60 seconds in
  all. The notification shows once each interval, the first time one interval
  from now.
- `on`: parts of a local date and time, `{ year?, month?, day?, weekday?,
  hour?, minute?, second? }`, with `month` 1 for January and `weekday` 1 for
  Sunday. Parts finer than the finest given one take their least value, so
  `{ hour: 9 }` matches 9:00:00, and coarser ones match any value. With
  `repeats`, the notification shows at every match. A match no time from now
  meets rejects with `TypeError`.
- `region` (iOS): `{ latitude, longitude, radius, onEntry?, onExit? }`, the
  radius in metres. The notification shows on entering the region, or as
  `onEntry` and `onExit` set, and with `repeats` each time. It needs location
  permission, which the location plugin asks for, and rejects with
  `NotAllowedError` without it. Other platforms reject with
  `NotSupportedError`.
- `exact` (Android): the alarm fires at the time rather than when Android
  next wakes, which while the device is idle can be minutes later. It needs
  the app to declare `SCHEDULE_EXACT_ALARM` in its Android manifest, and on
  Android 14 the user to allow it through `openSettings({ exactAlarms: true
  })`; without that, `schedule` rejects with `NotAllowedError`. Alarm and
  calendar apps can declare `USE_EXACT_ALARM` instead, which needs no user
  approval but which Google Play allows only for them.

`getScheduled` returns each notification with its trigger.

### Content

A notification has an `id`, a `title`, and optionally a `body`, `data`, a
`setup` `category` and, on Android, a `setup` `channel`. These fields apply
where their platforms have them:

| Field | Platforms | Meaning |
|---|---|---|
| `image` | All | An image, below: Apple's first attachment, Android's big picture, the web's `image` |
| `icon` | Android, web | An image beside the text: Android's large icon, the web's `icon` |
| `attachments` | Apple | Images, audio or video, below |
| `group` | Apple, Android | Shows the notifications of a group together: Apple's thread, Android's group under a summary the plugin posts and removes |
| `badge` | All | The app's badge count once the notification shows: Android's number, which launchers may show; the web's Badging API, for installed web apps only |
| `silent` | All | No sound or vibration |
| `sound` | Apple | `"default"`, `"defaultCritical"` or `{ name, critical?, volume? }`, below; Android plays the channel's sound |
| `timestamp` | Android, web | The time the notification shows, in milliseconds since the epoch |
| `subtitle` | Apple | A line under the title |
| `interruptionLevel` | Apple | `passive`, `active`, `timeSensitive` or `critical`; the last two need their entitlements |
| `relevanceScore` | Apple | From 0 to 1, how the notification ranks in the notification summary |
| `subText` | Android | A line with the app's name |
| `color` | Android | The accent colour, as `#rrggbb` |
| `style` | Android | `bigText`, `bigPicture`, `inbox` or `messaging`, below |
| `progress` | Android | `{ current, max }`, or `{ indeterminate: true }` |
| `ongoing` | Android | Keeps it from being dismissed; from Android 14, the user can still swipe it away on an unlocked device |
| `autoCancel` | Android | Whether opening it removes it; true by default |
| `onlyAlertOnce` | Android | A replacement does not alert again |
| `timeoutAfter` | Android | Removes it after this many milliseconds |
| `chronometer` | Android | Shows the time since `timestamp`, or until it with `{ countDown: true }` |
| `visibility` | Android | `public`, `private` or `secret`: how much shows on the lock screen |
| `publicVersion` | Android | `{ title, body }` the lock screen shows with `visibility` `private` |
| `sortKey` | Android | Orders it among others of its group |
| `androidCategory` | Android | `message`, `call`, `alarm`, `reminder`, `progress` and the other `Notification.CATEGORY_*` kinds, which ranking and Do Not Disturb use |
| `settingsText` | Android | The text of a link in the long-press menu that opens the app's notification settings, which `onOpenSettings` receives |
| `requireInteraction`, `renotify`, `vibrate`, `dir`, `lang` | Web | As `showNotification` takes them |
| `badgeImage` | Web | The URL of a small monochrome image that represents the app: `showNotification`'s `badge` |

Android's styles show the expanded notification:

```ts
await notifications.show({
  id: "chat-42",
  title: "Ada",
  body: "Are you coming tonight?",
  group: "supper-club",
  style: {
    type: "messaging",
    user: "Me",
    conversationTitle: "Supper club",
    messages: [
      { text: "Shall we book for eight?", at: Date.parse("2026-10-11T18:00:00Z") },
      { text: "Are you coming tonight?", at: Date.parse("2026-10-11T18:01:00Z"), sender: "Ada" },
    ],
  },
});
```

- `{ type: "bigText", text, title?, summary? }` shows more text.
- `{ type: "bigPicture", title?, summary?, showWhenCollapsed? }` shows
  `image`, which it requires. With `showWhenCollapsed`, the collapsed
  notification shows the image in place of the large icon.
- `{ type: "inbox", lines, title?, summary? }` shows a line of text for each
  of `lines`, as many as Android fits.
- `{ type: "messaging", user, conversationTitle?, messages }` shows a
  conversation. A message without a `sender` is the device's user's, and a
  `conversationTitle` makes it a group conversation.

On Apple platforms the badge stays until a notification sets another count.
On Android, a notification's number shows only while the notification does.

### Sounds

The `sounds` build setting names a directory of sound files the app ships:

```js
// vite.config.js
tokamak({ notifications: { sounds: "sounds" } });
```

It also takes `--notifications-sounds <dir>` and
`TOKAMAK_NOTIFICATIONS_SOUNDS`, and `notifications.ios.sounds` or
`--notifications-ios-sounds` for one platform. The build checks each file for
the platform it builds, and fails on one the platform cannot play:

- **Apple:** AIFF, WAV or CAF holding Linear PCM, IMA4, µLaw or aLaw, at most
  30 seconds long. A notification names a file with `sound: { name:
  "chime.wav" }`. Critical sounds, `"defaultCritical"` and `{ name, critical:
  true, volume? }`, play through the mute switch and Do Not Disturb, and need
  the critical alerts entitlement. Naming a file the app does not have rejects
  with `NotFoundError`.
- **Android:** WAV, MP3, Ogg, M4A, AAC or FLAC, named with lowercase letters,
  digits and underscores. A channel names a file with `sound: "chime.wav"`,
  and `setup` rejects with `NotFoundError` when the app does not have it.

A 16-bit PCM WAV with a lowercase name plays on both. APNs payloads name a
file in `aps.sound`; FCM notification messages name one without its extension
in `android.notification.sound`, as `"chime"`.

### Images

`image`, `icon`, `attachments` and actions' `icon` take an `https` URL or a
path the app serves, such as `/notifications/photo.jpg`, which loads as the
WebView would load it: a file the app ships or the Worker's response. Spaces
in either must be percent-encoded, as `%20`.

```ts
await notifications.show({
  id: "photo-7",
  title: "Grace",
  body: "Sent a photo",
  image: "/notifications/photo.jpg",
  icon: "https://example.com/avatars/grace.png",
});
```

In native builds the plugin loads them before it shows the notification, and
`show` resolves once it shows. One that does not load within 10 seconds, 8 on
Android, or that the platform cannot use is left out, and the notification
shows without it. A notification the Worker returns from `onPush` has what
remains of the time the platform allows for the message. A `remove` while the
images load keeps the notification from showing, and a later `show` with the
same `id` replaces it.

- On Apple platforms, `image` is the first attachment, and `attachments`
  adds others, each `{ url, hideThumbnail?, thumbnailClippingRect?,
  thumbnailTime? }`. The notification shows an attachment's thumbnail unless
  `hideThumbnail`: the part of an image `thumbnailClippingRect` gives as
  `{ x, y, width, height }` fractions, or a video's frame at `thumbnailTime`
  seconds. An attachment's type comes from the response's `Content-Type`, the
  path's extension or, for an image, its contents. Apple accepts JPEG, GIF and
  PNG images up to 10 MB, AIFF, WAV, MP3 and MPEG-4 audio up to 5 MB, and MPEG,
  MPEG-2, MPEG-4 and AVI video up to 50 MB.
- On Android, `image` shows when the notification is expanded, in the
  `bigPicture` style, or without a style; other styles leave it out. Images
  are PNG, JPEG, WebP or other files Android decodes, and an `https` image is
  at most 10 MB.
- On the web, the browser decides whether it shows `image` and `icon`.

An action's `icon` shows on Android wearables, not on phones, and in
Chrome.

FCM shows the image of a notification message itself while the app is off
screen, and the plugin shows it while the app is on screen. APNs
notifications show images only with a notification service extension, which
tokamak builds do not include.

## Push notifications

`subscribe()` subscribes to push messages and returns where the app's server
sends them. It returns the existing subscription when there is one. In native
builds it shows no UI, so the Worker can subscribe at every start, whether or
not the app starts on screen:

```ts
// src/tokamak.ts
import { onStart } from "@tokamakdev/tok/events";
import { notifications } from "@tokamakdev/plugin-notifications";

onStart(async (_event, _env, ctx) => {
  const subscription = await notifications(ctx).subscribe();
  await fetch("https://api.example.com/push-tokens", { method: "POST", body: JSON.stringify(subscription) });
});
```

| Platform | Subscription | Push service |
|---|---|---|
| iOS, macOS | `{ service: "apns", token, environment }` | APNs, in `environment` (`development` or `production`) |
| Android | `{ service: "fcm", fid }` | Firebase Cloud Messaging HTTP v1, sending to the Firebase Installation ID as the message's `fid`; during FCM's transition, `token` also accepts it |
| Web | `{ service: "webpush", endpoint, keys }` | Web Push, with the app's VAPID key pair |

On Android, `subscribeToTopic(topic)` and `unsubscribeFromTopic(topic)`
subscribe the installation to FCM topics, which the app's server sends to as
the message's `topic`. Topic names are 1 to 900 letters, digits and `-_.~%`.
Other platforms reject with `NotSupportedError`.

Push notifications that arrive while the page is visible reach `onMessage`,
and show as `showInForeground` sets.
`onSubscriptionChange` reports a replacement subscription, for example when FCM
registers a new Firebase Installation ID.

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

A Web Push payload is JSON: `{ "id": "...", "title": "...", "body": "...", "data": {}, "category": "..." }`,
with the web's content fields and `badge` as well. Every field is optional.
FCM notification messages carry Android's badge as
`android.notification.notification_count`.

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

## Opened and dismissed notifications

`onNotificationOpened` receives each notification the user opens, local or
push, and each action the user chooses. `action` is the action's `id`, absent
for the notification itself, and `input` is the text typed into the action,
absent without one. The plugin holds the latest 64 notifications opened before a page listens, including
the one that launched the app, and delivers them to the first listener.
Android does not pass the title or body of an FCM notification the system
showed, so those are absent.

`onNotificationDismissed` receives each notification the user dismisses, held
in the same way. Apple platforms report only notifications whose category has
`customDismissAction`. Android also reports notifications its `timeoutAfter`
removed, and does not report notifications FCM showed. The web reports only
notifications its service worker showed.

In native builds, the Worker's `onNotificationOpened` and
`onNotificationDismissed` listeners, from
`@tokamakdev/plugin-notifications/events`, also receive each notification,
whether or not the app is on screen:

```ts
// src/tokamak.ts
import { onNotificationOpened } from "@tokamakdev/plugin-notifications/events";

onNotificationOpened((opened, env, ctx) => {
  if (opened.action === "reply" && opened.input !== undefined) {
    ctx.waitUntil(env.REPLIES.put(opened.id, opened.input));
  }
});
```

## Web

The web implementation needs a service worker for everything but `show`. Add
the helpers to the app's own service worker:

```js
import {
  handleNotificationClick,
  handleNotificationClose,
  handlePush,
  handleSubscriptionChange,
} from "@tokamakdev/plugin-notifications/service-worker";

self.addEventListener("push", (event) => handlePush(event));
self.addEventListener("notificationclick", (event) => handleNotificationClick(event));
self.addEventListener("notificationclose", (event) => handleNotificationClose(event));
self.addEventListener("pushsubscriptionchange", (event) => handleSubscriptionChange(event));
```

- `handlePush` shows the payload's notification, with its category's actions,
  and forwards the message to open pages.
- `handleNotificationClick` holds the notification, or the chosen action, for
  `onNotificationOpened`. The notification itself and actions with
  `foreground` then focus an open page, or open `data.url` (default `/`). It
  returns false for notifications the plugin did not show.
- `handleNotificationClose` holds a notification the user dismissed for
  `onNotificationDismissed`. It returns false for notifications the plugin did
  not show.
- `handleSubscriptionChange` forwards a replacement subscription to open pages.

Safari supports Web Push on macOS 13 and later, and on iOS and iPadOS 16.4 and
later only for web apps added to the Home Screen.

## Android icon

Android shows notifications with a monochrome icon. Add one as
`drawable/tokamak_notification` in the app's Android resource directory, set
with `android.icon`. Without it, the plugin uses the launcher icon, which
Android draws as a solid shape.

## Errors

- `NotAllowedError`: permission has not been granted, including the exact
  alarm and location permissions some triggers need.
- `NotFoundError`: a sound the app does not ship.
- `NeedsUIError`: `requestPermission()` would show the prompt while the app is
  off screen on iOS or Android.
- `NotSupportedError`: the platform or build cannot perform the call.
- `InvalidStateError`: the app has not made a declaration push requires.
- `QuotaExceededError`: the platform's limit on scheduled notifications is
  reached.
- `OperationError`: a platform or push service failure.
- `TypeError`: an invalid argument.
