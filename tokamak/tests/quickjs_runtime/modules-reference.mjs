import { readFileSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { Miniflare, convertV4MiniflareOptions } from "miniflare";

const root = join(import.meta.dirname, "modules");
const state = await mkdtemp(join(tmpdir(), "tokamak-modules-"));
process.chdir(state);
const module = (type, name) => ({ type, path: join(root, name), contents: readFileSync(join(root, name)) });
const mf = new Miniflare(convertV4MiniflareOptions({
  modulesRoot: root,
  cf: false,
  compatibilityDate: "2026-08-25",
  compatibilityFlags: ["nodejs_compat"],
  modules: [
    module("ESModule", "modules.mjs"),
    module("Text", "text.txt"),
    module("Text", "bom.txt"),
    module("Text", "invalid.txt"),
    module("Data", "data.bin"),
  ],
}));
try {
  const response = await mf.dispatchFetch("http://localhost/");
  if (!response.ok) throw new Error(await response.text());
  console.log(await response.text());
} finally {
  await mf.dispose();
  await rm(state, { recursive: true, force: true });
}
