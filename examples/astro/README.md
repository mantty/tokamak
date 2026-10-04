# tokamak Astro Example

Astro SSR app using the Cloudflare adapter. It exercises server rendering, one
prerendered route, asset serving, navigation, and a WebSocket endpoint.

`astro.config.mjs` adds the tokamak Vite plugin, and `src/tokamak.ts` holds the
app's tokamak configuration. The example installs `@tokamakdev/tok` from the
workspace, so build that package first. From this directory:

```sh
npm ci --prefix ../../tokamak-cli/npm
npm run build --prefix ../../tokamak-cli/npm
pnpm --dir ../../plugins install --frozen-lockfile
pnpm install --frozen-lockfile
pnpm run build
```

Build a platform pack from the tokamak workspace, then package the app, which
also builds it:

```sh
cargo run -p xtask -- platform-pack --target macos-arm64
TOKAMAK_PLATFORM_PACK_PATH=../../target/tokamak-platform-packs \
  cargo run -p tokamak-cli -- build macos --project .
```

The example intentionally uses no WebAssembly. It covers server rendering,
static assets, API routes, navigation, and WebSockets through the QuickJS
runtime.
