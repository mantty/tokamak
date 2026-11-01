// What the reference scripts share.
import { readFileSync } from "node:fs";
import { stripTypeScriptTypes } from "node:module";
import { join } from "node:path";

/** The ES module of the TypeScript file `name` under `root`, as Miniflare
 * takes it, with its types removed as tokamak's tests remove them. */
export function esModule(root: string, name: string) {
  const path = join(root, name);
  return { type: "ESModule", path, contents: stripTypeScriptTypes(readFileSync(path, "utf8")) } as const;
}
