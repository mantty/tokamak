# tokamak development mode

Status: active. The core development loop is implemented; this document is a
delta plan and intentionally records only the work that remains.

## 1. Bridge backend plugin calls during development

Frontend plugin calls already use the device WebView bridge. Backend plugin
calls remain outstanding because development Worker code executes in the host
Cloudflare-compatible runtime:

```text
host Worker binding/service
    -> host-side tokamak bridge
    -> development relay
    -> device runtime
    -> native plugin implementation
```

Implement the host binding/service and the device endpoint as one narrow,
authenticated contract. It must:

- identify the app, device, and development session;
- carry JSON values and binary payloads;
- support the plugin's required streaming or long-running calls;
- propagate cancellation, timeouts, disconnects, and native errors; and
- return the same unsupported-capability error semantics as the existing
  frontend plugin bridge.

The browser must never receive the backend capability credential. Backend
plugin JavaScript must remain valid Cloudflare Worker code; QuickJS-only code
must fail in the host development runtime rather than creating a second
device-side module-execution path.

## 2. Recover transient development-session failures

The device proxy retries safe HTTP loads for ten seconds when the stable relay
disconnects during a framework-server restart. The host-side supervisor still
has no recovery contract for a persistent host-server or relay failure. The
mobile shells already probe and restore the device gateway when they resume.
Add recovery that:

- keeps the selected target, installed app, relay endpoint, and session
  credentials unchanged while the host server is unavailable;
- waits for the configured framework endpoint to return and resumes new
  requests without reinstalling the app;
- lets the framework's HMR client reconnect or reload the WebView when an HMR
  connection was interrupted; and
- distinguishes a temporary failure from an intentional command exit or user
  shutdown, reporting persistent failures and stopping without orphaned
  processes or silently selecting another target.

Physical iOS signing uses the same split as Xcode: the Apple platform pack selects
an exact valid profile when one is already installed, and otherwise asks
`xcodebuild` to update or register the profile automatically for the selected
bundle ID and device. Set `TOKAMAK_IOS_SIGNING_IDENTITY` together with
`TOKAMAK_IOS_PROVISIONING_PROFILE` when an explicit signing choice is required.

## 3. Rebuild for native-input changes

The initial development build, installation, and launch are implemented. The
remaining lifecycle work is to watch native shell and native-plugin inputs
while the framework process continues running.

When such an input changes, tokamak should rebuild the same platform pack,
reinstall/relaunch the selected target, and preserve the existing framework
server, relay, and target selection. JavaScript, styles, public assets, and
supported Worker edits must continue using the framework's HMR/reload path
without a native rebuild.

## 4. End-to-end coverage

Add representative tests and manual checks for the implemented path and the
remaining work above:

- Astro with the Cloudflare integration, including style/module HMR, SSR/API
  requests, and browser WebSockets through the device WebView;
- plain Vite plus the Cloudflare Worker environment;
- the agreed vinext/Next Cloudflare setup;
- host targets on macOS, Windows, and Linux where a platform pack exists;
- iOS Simulator, physical iOS, Android emulator, and supported physical
  Android targets;
- unsupported Wrangler bindings producing one readable warning block while
  development continues;
- temporary host-server or relay loss recovering on the existing session;
- backend native plugin calls once the bridge is available; and
- native-input rebuild/reinstall without restarting the framework process.

The tests must continue to assert the important split: tok dev uses the
framework's host Cloudflare runtime and normal HMR machinery, while `tok build`
and packaged-runtime tests exercise the packaged QuickJS runtime.

## References

- [Cloudflare Vite plugin](https://developers.cloudflare.com/workers/vite-plugin/)
- [Cloudflare local development](https://developers.cloudflare.com/workers/local-development/)
- [Astro Cloudflare adapter](https://docs.astro.build/en/guides/integrations-guide/cloudflare/)
- [Cloudflare Next.js guide](https://developers.cloudflare.com/workers/framework-guides/web-apps/nextjs/)
