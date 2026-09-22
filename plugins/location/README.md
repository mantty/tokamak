# @tokamakdev/plugin-location

Location for Tokamak applications and the web.

Add it to your project's `dependencies`; `tok` does not include plugins listed
only in `devDependencies`:

```sh
npm install @tokamakdev/plugin-location@beta
```

```ts
import { location } from "@tokamakdev/plugin-location";

const position = await location.getCurrentPosition();

const stop = location.watchPosition(
  (next) => console.log(next.coords.latitude, next.coords.longitude),
  console.error,
);
```

The web implementation uses `navigator.geolocation`. Native Tokamak builds use
Core Location on Apple platforms, `LocationManager` on Android, and WebView2's
location implementation on Windows. Calling the API on a platform without
location support throws `NotSupportedError`.
