# @tokamakdev/plugin-location

Location for Tokamak applications and the web.

Add it to your project's dependencies:

```sh
npm install @tokamakdev/plugin-location
```

```ts
import { location } from "@tokamakdev/plugin-location";

const position = await location.getCurrentPosition();

const stop = location.watchPosition(
  (next) => console.log(next.coords.latitude, next.coords.longitude),
  console.error,
);
```

`getCurrentPosition` takes the Geolocation API's `maximumAge` and `timeout`
options. It resolves with a recent position no older than `maximumAge`
milliseconds, 0 by default, or takes a new one, and rejects with
`TimeoutError` when none arrives within `timeout` milliseconds, unlimited by
default. The timeout starts once the user allows location.

## Off screen

The plugin asks for location while the app is in use only. While the app is
off screen on iOS or Android, as when Worker code runs in the background, a
call that would ask the user for permission rejects with `NeedsUIError`. With
permission granted, the platforms stop locating an app that is off screen
without background location permission, which the plugin does not request, so
`getCurrentPosition` rejects with `NotAllowedError` there. iOS still locates
the app for a few seconds after it leaves the screen.

## Platforms

The web implementation uses `navigator.geolocation`. Native Tokamak builds use
Core Location on Apple platforms, `LocationManager` on Android, and WebView2's
location implementation on Windows. Calling the API on a platform without
location support throws `NotSupportedError`.
