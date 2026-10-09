# tokamak Astro Example

Astro SSR app using the Cloudflare adapter. It exercises server rendering, one
prerendered route, asset serving, navigation, a WebSocket endpoint, and
lifecycle events: `src/tokamak.ts` records each start, resume and suspend in
the `EVENTS` KV namespace, and, when the app starts on screen, where the
location plugin first places the device. The Events page lists them, with the
app's lifecycle stage.

`astro.config.mjs` adds the tokamak Vite plugin with the app's tokamak
configuration. The example installs `@tokamakdev/tok` from the workspace, so
build that package first. From this directory:

```sh
npm ci --prefix ../../tokamak-cli/npm
npm run build --prefix ../../tokamak-cli/npm
pnpm --dir ../../plugins install --frozen-lockfile
pnpm install --frozen-lockfile
```

`pnpm run build` builds the web app. Build a platform pack from the tokamak
workspace, then package the app, which also builds it:

```sh
cargo run -p xtask -- platform-pack --target macos-arm64
TOKAMAK_PLATFORM_PACK_PATH=../../target/tokamak-platform-packs \
  cargo run -p tokamak-cli -- build macos --project .
```

The example intentionally uses no WebAssembly. It covers server rendering,
static assets, API routes, navigation, and WebSockets through the QuickJS
runtime.
