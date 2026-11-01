import { readFileSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { Miniflare, convertV4MiniflareOptions } from "miniflare";
import { esModule } from "./reference.ts";

const root = import.meta.dirname;
const state = await mkdtemp(join(tmpdir(), "tokamak-workerd-"));
process.chdir(state);
const mf = new Miniflare(convertV4MiniflareOptions({
  modulesRoot: root,
  cf: false,
  compatibilityDate: "2026-08-25",
  compatibilityFlags: ["nodejs_compat"],
  bindings: { FLAG: "enabled" },
  modules: [...["startup.ts", "contracts.ts", "streams.ts", "crypto.ts", "html.ts", "zlib.ts", "clone.ts", "performance.ts", "intl.ts", "bytes.ts", "filesystem.ts", "net.ts", "events.ts", "outcome.ts"].map(name => esModule(root, name)),
    { type: "Text", path: `${root}/bundle.txt`, contents: readFileSync(`${root}/bundle.txt`, "utf8") }],
}));
try {
  const response = await mf.dispatchFetch("http://localhost/");
  if (!response.ok) throw new Error(await response.text());
  console.log(await response.text());
} finally {
  await mf.dispose();
  await rm(state, { recursive: true, force: true });
}
