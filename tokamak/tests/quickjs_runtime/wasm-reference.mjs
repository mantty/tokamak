import { readFileSync } from "node:fs";
import { join } from "node:path";
import { Miniflare, convertV4MiniflareOptions } from "miniflare";
const root = process.argv[2];
const module = (type, name) => ({ type, path: join(root, name), contents: readFileSync(join(root, name)) });
const mf = new Miniflare(convertV4MiniflareOptions({
  modulesRoot: root, cf: false, compatibilityDate: "2026-08-25",
  modules: [module("ESModule", "contracts.mjs"), module("CompiledWasm", "core.wasm")],
}));
try {
  const response = await mf.dispatchFetch("http://localhost/");
  if (!response.ok) throw new Error(await response.text());
  console.log(await response.text());
} finally { await mf.dispose(); }
