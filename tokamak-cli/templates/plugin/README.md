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

The root SDK runs only with a Tokamak context. Browser implementations, native
platform sources, events, and higher-level wrappers are optional. A browser
fallback belongs in a separate module referenced by `web` in the manifest; it
is used only by the generated frontend client in a web deployment.
