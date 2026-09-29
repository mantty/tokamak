import { readFileSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { Miniflare, convertV4MiniflareOptions } from "miniflare";

const root = import.meta.dirname;
const state = await mkdtemp(join(tmpdir(), "tokamak-storage-"));
process.chdir(state);
const mf = new Miniflare(convertV4MiniflareOptions({
  modulesRoot: root,
  cf: false,
  compatibilityDate: "2026-08-25",
  compatibilityFlags: ["nodejs_compat"],
  d1Databases: ["DB"],
  d1Persist: join(state, "d1"),
  kvNamespaces: ["KV"],
  kvPersist: join(state, "kv"),
  modules: [{ type: "ESModule", path: `${root}/storage.mjs`, contents: readFileSync(`${root}/storage.mjs`, "utf8") }],
}));
try {
  const response = await mf.dispatchFetch("http://localhost/");
  if (!response.ok) throw new Error(await response.text());
  console.log(await response.text());
} finally {
  await mf.dispose();
  await rm(state, { recursive: true, force: true });
}
