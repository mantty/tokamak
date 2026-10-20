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
| `@tokamakdev/tok` | `TokamakEntrypoint`, `TokamakContext`, `TokamakEvent`, and the Vite plugin's `Config` type |
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
extends `TokamakEntrypoint`; public methods are checked against Tokamak's
RPC contract. The main worker calls it through `env.TOKAMAK`; the native worker
calls the main worker through `env.MAIN`. Development delivers native events
through Cloudflare's dev registry and gives only the native worker the device
relay. Vite reloads its modules and listeners when their source changes.
Run `wrangler types` to generate the project's Workers declarations before
building or starting development.

Tokamak generates `tokamak-env.d.ts` in the project root, refreshing it during
development. Include it in the project's TypeScript inputs alongside
`worker-configuration.d.ts`, and ignore the generated file in version control.
It adds the optional
`Cloudflare.Env.TOKAMAK` binding and exports `TokamakService` and `TokamakClient`
types from `virtual:tokamak/client`. Service methods return ordinary promises,
preserve the entrypoint's overloads, and expose only its public RPC operations.
`Tokamak.Env` describes the native worker's required `MAIN` service plus its
shared variables, KV, D1, and R2 bindings. `this.env` in a `TokamakEntrypoint`
and the `env` argument of native listeners use this type.

Page code imports the generated client as the module's default export:

```ts
// src/tokamak.ts
import { TokamakEntrypoint } from "@tokamakdev/tok";

export default class Native extends TokamakEntrypoint {
  greeting(name: string): string {
    return `Hello, ${name}`;
  }
}
```

```ts
// Page code
import native from "virtual:tokamak/client";

const message = await native.greeting("Ada");
```

Native clients call the Tokamak worker through the reserved `/_tokamak/app`
route. The native worker also receives other requests under `/_tokamak/` in
its `fetch` handler. Arguments and results support the RPC value and readable
byte-stream contract. Stream cancellation reaches its producer, and leaving
the page closes its calls. Web builds expose the same app-method names, rejecting calls with
`NotSupportedError`. The client bundle contains the callable interface and
transport.

The entrypoint receives its own bindings in `this.env` and native invocation
context in `this.ctx`. Its default `event()` runs registered listeners; an
override receives `{ type, data }` and can return a reply or promise. Native
events use this handler independently of page requests. `event()` is excluded
from the service binding's public methods. Listener callbacks receive the same
invocation context and register follow-up work with `ctx.waitUntil()`.

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
