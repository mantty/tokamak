# tokamak-plugin-{{id}}

Install this package in your app with npm, pnpm, or Yarn. During development,
use a workspace dependency or `npm install ../{{id}}`; publishing after each
edit is unnecessary.

In `src/tokamak.ts`:

```ts
import { TokamakEntrypoint } from "@tokamakdev/tok";
import { {{factory}} } from "tokamak-plugin-{{id}}";

export default class Native extends TokamakEntrypoint {
  example() {
    return {{factory}}(this.ctx).{{operation}};
  }
}
```

In page code:

```ts
import api from "tokamak-plugin-{{id}}/client";
console.log(await api.{{operation}});
```

Tokamak generates the frontend client, service binding, and declarations from
the factory's typed methods. Run the app's usual development command to refresh
types and clients. TypeScript edits reload; native code edits require rebuilding.

Run `npm install` and `npm run check` in this package. Use `npm pack --dry-run`
to inspect the package, then publish with the normal `npm publish` workflow.
Rename the npm package to your own scope before publishing if needed.

The root SDK runs only with a Tokamak context. Everything else is optional:

- `web/index.ts`, which the manifest's `web` references, implements operations
  in the browser for the generated client in a web deployment. Remove it, and
  the manifest entry, if the plugin is native-only.
- `src/events.ts`, exported as `tokamak-plugin-{{id}}/events`, exports the
  functions that register listeners of the native events. A plugin for more
  families registers each listener there on every family's event.
- `apple/` and `android/` hold native code, which builds for the manifest's
  `platforms`. `Exports.swift` or `Exports.kt` lists the methods the Worker
  calls and the events native code emits.
- `src/apple.generated.ts` and `src/android.generated.ts` are each family's
  TypeScript API, which `npm run generate:apple` or `npm run generate:android`
  writes from the registration file, with the family's native dispatch.
  `platformApi` in `src/index.ts` picks the running platform's. Commit the
  generated files: `npm run check` fails when they differ from what the
  installed toolchains generate.
