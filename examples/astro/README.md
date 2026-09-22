# tokamak Astro Example

Astro SSR app using the Cloudflare adapter. It exercises server rendering, one
prerendered route, asset serving, navigation, and a WebSocket endpoint.

From this directory, build the web app with:

```sh
pnpm --dir ../../plugins install --frozen-lockfile
pnpm install --frozen-lockfile
pnpm run build
```

Build a platform pack from the tokamak workspace, then package it:

```sh
cargo run -p xtask -- platform-pack --target macos-arm64
TOKAMAK_PLATFORM_PACK_PATH=../../target/tokamak-platform-packs \
  cargo run -p tokamak-cli -- build macos --project . --wrangler dist/server/wrangler.json
```

The example intentionally uses no WebAssembly. It covers server rendering,
static assets, API routes, navigation, and WebSockets through the QuickJS
runtime.

## Packaged startup regression

The startup fixture uses text encoding at module scope before loading the Astro
Worker. Run it in a packaged simulator app, not `tok dev`:

```sh
# From the tokamak workspace:
cargo build -p tokamak-cli --release
cargo run -p xtask -- platform-pack --target ios-simulator-arm64
cd examples/astro
pnpm build
TOKAMAK_PLATFORM_PACK_PATH=../../target/tokamak-platform-packs \
  ../../target/release/tok build ios-simulator \
  --wrangler wrangler.runtime-test.json --skip-project-build
xcrun simctl install booted build/ios-simulator/tokamak-example-astro.app
xcrun simctl launch --terminate-running-process booted com.tokamak.tokamak-example-astro
```

The app displays the normal Astro page. The fixture is separate from the example's
default Wrangler configuration.
