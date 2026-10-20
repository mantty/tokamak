# @tokamakdev/tok

The tokamak CLI, Vite plugin and app API, published from the tokamak
workspace. The version is the workspace Cargo version, with a `-beta.N` suffix
for automated pre-releases.

```sh
npm install --save-dev @tokamakdev/tok
npx tok targets
```

| Entry | Contents |
| --- | --- |
| `@tokamakdev/tok` | The `Config` type of the Vite plugin's options |
| `@tokamakdev/tok/events` | The app's events, `onStart`, `onResume` and `onSuspend`, and `getLifecycleStage()` |
| `@tokamakdev/tok/vite` | The Vite plugin, `tokamak()` |
| `@tokamakdev/tok/astro` | The Astro adapter, `tokamak()` |
| `tok` | The CLI |

The Vite integration includes Cloudflare's plugin. Its options are the app's
tokamak configuration; the second argument accepts Cloudflare plugin options:

```ts
import { tokamak } from "@tokamakdev/tok/vite";
import { defineConfig } from "vite";

export default defineConfig({
  plugins: [tokamak({ name: "My App", version: "1.0.0" })],
});
```

In Astro, use `tokamak()` from `@tokamakdev/tok/astro` as the `adapter`, passing
Cloudflare adapter options as the second argument.

Web builds use the ordinary Cloudflare integration. When `tok` runs the
command, `src/tokamak.ts` becomes a separate native worker. Its default export
is a `WorkerEntrypoint` subclass; public methods are checked against Tokamak's
RPC contract. The main worker calls it through `env.TOKAMAK`; the native worker
calls the main worker through `env.MAIN`. Development delivers native events
through Cloudflare's dev registry and gives only the native worker the device
relay. Vite reloads its modules and listeners when their source changes.
Run `wrangler types` to generate the project's Workers declarations before
building or starting development.

The package installs the `tok` binary for the current machine through an
optional dependency (`@tokamakdev/tok-darwin-arm64`, `-darwin-x64`,
`-linux-x64`, or `-win32-x64`), and the platform packs the machine can build
as `@tokamakdev/platform-<target>` optional dependencies:

| Machine | Platform packs |
| --- | --- |
| macOS, Apple silicon | `android-arm64`, `ios-arm64`, `ios-simulator-arm64`, `macos-arm64` |
| macOS, Intel | `android-arm64`, `ios-arm64`, `ios-simulator-x64`, `macos-x64` |
| Windows x64 | `android-arm64`, `windows-x64` |
| Linux x64 | `android-arm64` |

The launcher passes the installed platform packs to `tok` in
`TOKAMAK_PLATFORM_PACK_PATH`, unless that variable is already set.
