# @tokamakdev/plugin-location

Location for tokamak applications and the web: positions on request,
long-term location awareness from significant changes, visits and regions,
and continuous tracking, all of which reach the app's Worker off screen.

```sh
npm install @tokamakdev/plugin-location
```

```ts
import { location } from "@tokamakdev/plugin-location";

const position = await location.getCurrentPosition();
const stop = location.watchPosition((next) => console.log(next.coords), console.error);

await location.requestPermission({ background: true });
await location.addRegion({ id: "home", latitude: 51.5, longitude: -0.12, radius: 150 });
```

```ts
// src/tokamak.ts
import { location } from "@tokamakdev/plugin-location";
import { onLocationUpdate, onRegionTransition } from "@tokamakdev/plugin-location/events";
import { onStart } from "@tokamakdev/tok/events";

onStart(async () => {
  await location.setup({ significantChanges: true, visits: true });
});
onLocationUpdate(async ({ positions }) => {
  await fetch("https://example.com/whereabouts", { method: "POST", body: JSON.stringify(positions) });
});
onRegionTransition(({ id, transition }) => console.log(id, transition));
```

## Positions

`getCurrentPosition` takes the Geolocation API's `maximumAge` and `timeout`
options. It resolves with a recent position no older than `maximumAge`
milliseconds, 0 by default, or takes a new one, and rejects with
`TimeoutError` when none arrives within `timeout` milliseconds, unlimited by
default. The timeout starts once the user allows location.

`watchPosition(next, error)` calls `next` with each new position until the
function it returns is called.

Both ask for in-use permission when the user has not answered. Off screen
without background permission, the platforms stop locating the app, so
`getCurrentPosition` rejects with `NotAllowedError` there. iOS still locates
the app for a few seconds after it leaves the screen. With approximate
permission, Android gives an app a new position at most every 10 minutes, so
`getCurrentPosition` can wait that long.

## Permission

`permission()` reports the app's permission without asking:

- `inUse` and `background`, each `"granted"`, `"denied"` or `"prompt"`.
  `background` is `"denied"` unless the `background` build setting declares
  it. macOS has no separate background permission, so `background` equals
  `inUse` there.
- `precision`: `"precise"` or `"approximate"`, what the app gets or would get
  once allowed.

`requestPermission(options)` asks for in-use permission where the user has not
answered, and for what `options` names, then resolves with the permission:

- `background: true` asks for background permission.
  - **iOS** shows the Always prompt once, from When In Use. Asked before the
    user has answered, it gives provisional Always, which iOS confirms with
    the user later. Without the `background` build setting it rejects with
    `NotSupportedError`.
  - **Android** asks for in-use permission first, as Android grants
    background permission only after it. From Android 11 the request opens
    the app's page in Settings, where the user picks "Allow all the time".
- `precision: "precise"` asks for precise location where the user allowed
  approximate. Apple platforms grant it temporarily, while the app is in use,
  for the reason the `accuracy-purposes` build setting gives for `purpose`.
  `precision: "approximate"` asks Android for approximate location only.

Off screen on iOS and Android, a request that would show a prompt rejects with
`NeedsUIError`. `openSettings()` opens the app's page in the system settings,
or Location Services on macOS, where the user changes the permission.
`servicesEnabled()` reports whether location services are on for the device.

## Build settings

The plugin's declarations come from its build settings, set in the tokamak
Vite plugin's options, as `--location-<key>`, or as `TOKAMAK_LOCATION_<KEY>`:

```js
// vite.config.js
tokamak({
  location: {
    background: true,
    "background-updates": true,
    "accuracy-purposes": { Directions: "Shows directions from where you are" },
  },
});
```

| Setting | Kind | Declares |
|---|---|---|
| `background` | boolean | Background permission: iOS's Always reason, Android's `ACCESS_BACKGROUND_LOCATION`. |
| `background-updates` | boolean | Updates off screen: iOS's `location` background mode, Android's location foreground service and its permissions. |
| `precision` | `precise` or `approximate` | `approximate` declares approximate location only: iOS's `NSLocationDefaultAccuracyReduced`, and Android without `ACCESS_FINE_LOCATION`. macOS has no such declaration. |
| `when-in-use-description` | string | Apple: the in-use prompt's reason, and macOS's only reason. |
| `always-description` | string | iOS: the Always prompt's reason. Needs `background`. |
| `accuracy-purposes` | map of string | Apple: the reasons for temporary precise location, by purpose key. |

Without a description the plugin uses a generic English reason. Declare only
what the app uses: App Review rejects background modes an app has no feature
for, and Google Play reviews background location and foreground service types
with a declaration and a video.

## Worker events

The Worker's listeners, from `@tokamakdev/plugin-location/events`, run whether
or not a page is loaded, including when the system launches the app for an
event:

| Event | Receives |
|---|---|
| `onLocationUpdate` | `{ positions, stationary? }` from updates and significant changes. |
| `onRegionTransition` | `{ id, transition, timestamp }`, where `transition` is `"enter"` or `"exit"`. |
| `onVisit` | Apple only: `{ latitude, longitude, accuracy, arrival, departure }`, times in milliseconds or null when unknown. |
| `onLocationError` | `{ name, message, id? }` when a background service fails, with the region's `id` when it concerns one. |

`positions` holds the positions that arrived while the Worker handled the
previous update, oldest first, so a slow Worker never builds a queue. Each is
newer than those delivered before it, so a position that both updates and
significant changes report arrives once. Apple's `stationary`, from iOS 18
and macOS 15, is true on the last update before updates pause while the
device is still.

The plugin's `onLocationUpdate`, `onRegionTransition`, `onVisit` and
`onLocationError` methods receive the same events while the listening page or
Worker runs, and return the function that removes the listener.

## Long-term awareness

`setup(config)` keeps the app aware of where the device is over days, with
little power and no indicator. Call it from `onStart` in `src/tokamak.ts`, so
it applies at every launch; the plugin also keeps it and applies it again
itself. The configuration replaces the previous one, and an invalid one
rejects with `TypeError` and changes nothing. Apple platforms reject with
`NotSupportedError` where the device cannot monitor significant changes.

- `significantChanges` delivers moves of 500 metres or more, at most every 5
  minutes, to `onLocationUpdate`, located from cellular and Wi-Fi rather than
  GPS. Apple platforms use the significant-change service. Android uses a
  low-power request with the same thresholds, which it limits to a few an hour
  off screen. Without permission they start once the app has it on Apple
  platforms, and on Android after the plugin's prompt or at the next launch.
- `visits`, Apple only, delivers arrivals at and departures from the places
  the user stays to `onVisit`. It uses the least power of all.

With background permission, iOS relaunches the app for both, and Android starts
the app's process for significant changes. [Regions](#regions) wake the app
the same way.

## Continuous tracking

`startUpdates(options)` starts continuous updates for an activity the user
follows, such as a run or navigation, replacing any running, until
`stopUpdates()`. Off screen the system can show that the app is tracking:
iOS's blue location pill with in-use permission, and the notification of
Android's foreground service.
`getUpdates()` resolves with the running updates' options, or null. Updates
need in-use permission, and reject with `NotAllowedError` without it. The
plugin keeps them across launches and restarts.

**Apple** uses `CLLocationUpdate.liveUpdates`.

- `activity` is the live configuration: `"default"`, `"automotiveNavigation"`,
  `"otherNavigation"`, `"fitness"`, `"airborne"`, or `"maritime"` (iOS 27 and
  macOS 27). It tunes when updates pause and how coordinates follow roads.
- Updates pause while the device is still and resume when it moves.
- If iOS ends the app while updates run, it launches the app for the next
  update, and the plugin resumes them until `stopUpdates()`. With Always
  permission, swiping the app away does not stop them either.
- Off screen, updates need Always permission, or the `background-updates`
  build setting, whose background activity session shows the pill.
  `startUpdates` rejects with `NeedsUIError` off screen on iOS.
- From iOS 18, failures such as denied permission or lost location reach
  `onLocationError` once each.
- `getCurrentPosition` and `watchPosition` can change the coordinates updates
  deliver while they run.

**Android** uses `LocationManager` with a `PendingIntent`, which starts the
app's process for each update.

- `accuracy` is `"high"` (default), `"balanced"` or `"low"`; `distance` the
  metres between updates, 0 by default; `interval` the milliseconds between
  them, 10 seconds by default.
- Off screen, updates need background permission and arrive a few times an
  hour, unless they run in a foreground service.
- `foregroundService: { title, body, channel }` runs updates in a foreground
  service, with that notification in a channel of that name, at the requested
  rate off screen. It needs the `background-updates` build setting, and
  rejects with `NotSupportedError` without it. Off screen it needs background
  permission, and Android starts the service only within 10 seconds of an
  update; otherwise it rejects with `NeedsUIError`. A region transition does
  not allow the start.
- The plugin requests updates again after a restart, an app update and a
  force stop, and starts the service again on screen, or after a restart or
  update with background permission. Android restarts a service it ends for
  memory.

## Regions

`addRegion({ id, latitude, longitude, radius, onEntry, onExit })` monitors a
circular region of `radius` metres, replacing one with the same `id`, until
`removeRegion(id)`. `onEntry` and `onExit` are true by default. Entries and
exits reach `onRegionTransition`, including when the system launches the app
for one. A region starts outside, so a device already inside reports an
entry. `getRegions()` resolves with the regions, each with its `state` on
Apple platforms: `"inside"`, `"outside"` or `"unknown"`.

Regions need precise location, and reject with `NotAllowedError` without it,
and background permission off screen.

- **Apple** uses `CLMonitor`, which monitors up to 20 regions; `addRegion`
  rejects with `QuotaExceededError` beyond them. iOS relaunches the app for a
  transition, even after the user quits it. macOS monitors only while the app
  runs.
- **Android** uses proximity alerts, which the system evaluates about every 30
  minutes or less often, so a transition can arrive that long after it
  happens. Adding a region again, as the plugin does after a restart, reports
  an entry again for a device inside.

## Platforms

The web uses `navigator.geolocation`. `permission()` reports `inUse` from the
Permissions API, and the other additions reject with `NotSupportedError`,
while the listener methods receive nothing. Windows uses WebView2's location
for `getCurrentPosition` and `watchPosition`.

## Errors

Rejections and `onLocationError` events are `DOMException` names:
`NotAllowedError` (no permission, or none for the background),
`NotReadableError` (location unavailable, or services off), `NeedsUIError` (a
prompt or a start off screen), `NotSupportedError`, `QuotaExceededError`,
`TimeoutError` and `TypeError`.
