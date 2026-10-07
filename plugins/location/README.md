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

The web implementation uses `navigator.geolocation`. Native Tokamak builds use
Core Location on Apple platforms, `LocationManager` on Android, and WebView2's
location implementation on Windows. Calling the API on a platform without
location support throws `NotSupportedError`.
