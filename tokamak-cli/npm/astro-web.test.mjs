import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { test } from "node:test";
import { fileURLToPath, pathToFileURL } from "node:url";
import { build } from "astro";
import { project } from "./astro-fixture.mjs";

test("Astro generates inferred native client types for a web build", async t => {
  const { root } = project(t, { sessionKVBindingName: "APP_SESSION" });
  const sdk = JSON.stringify(fileURLToPath(import.meta.resolve("@tokamakdev/tok")));
  fs.writeFileSync(path.join(root, "src/tokamak.ts"), `import { TokamakEntrypoint } from ${sdk};
    export default class Native extends TokamakEntrypoint {
      async describe() {
        const session = await this.env.APP_SESSION.get("description");
        return session ?? (await this.env.MAIN.fetch("https://app.example")).text();
      }
    }`);
  fs.writeFileSync(path.join(root, "src/pages/index.astro"), `<h1>Web client</h1>
    <script>
      import client from "virtual:tokamak/client";
      document.addEventListener("native-description", () => client.describe().catch(console.error));
    </script>`);
  await build({ root: pathToFileURL(`${root}/`), logLevel: "silent" });
  const pointer = path.join(root, ".wrangler/deploy/config.json");
  const deploy = JSON.parse(fs.readFileSync(pointer, "utf8"));
  assert.deepEqual(deploy.auxiliaryWorkers ?? [], []);
  assert.ok(fs.existsSync(path.join(root, "tokamak-env.d.ts")));
});
