# tokamak Astro Example

Astro SSR app using the Cloudflare adapter. It exercises server rendering, one
prerendered route, asset serving, navigation, and a WebSocket endpoint.

From this directory, build the web app with:

```sh
pnpm --dir ../../plugins install --frozen-lockfile
pnpm install --frozen-lockfile
pnpm run build
```

Build a target pack from the tokamak workspace, then package it:

```sh
cargo run -p xtask -- target-pack --target macos-arm64
TOKAMAK_TARGET_PACK_DIR=../../target/tokamak-target-packs \
  cargo run -p tokamak-cli -- build macos --project . --wrangler dist/server/wrangler.json
```

The example intentionally uses no WebAssembly. It covers server rendering,
static assets, API routes, navigation, and WebSockets through the QuickJS
runtime.
