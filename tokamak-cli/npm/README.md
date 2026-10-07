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
| `@tokamakdev/tok/vite` | The Vite plugin, `tokamak()` |
| `tok` | The CLI |

The Vite plugin goes next to Cloudflare's, and its options are the app's
tokamak configuration:

```ts
import { cloudflare } from "@cloudflare/vite-plugin";
import { tokamak } from "@tokamakdev/tok/vite";
import { defineConfig } from "vite";

export default defineConfig({
  plugins: [cloudflare(), tokamak({ name: "My App", version: "1.0.0" })],
});
```

It does nothing unless `tok` runs the build or development command.

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
