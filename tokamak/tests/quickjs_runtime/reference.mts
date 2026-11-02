// What the reference scripts share.
import { readFileSync } from "node:fs";
import type { Server } from "node:http";
import { stripTypeScriptTypes } from "node:module";
import { join } from "node:path";

/** The ES module of the TypeScript file `name` under `root`, as Miniflare
 * takes it, with its types removed as tokamak's tests remove them. */
export function esModule(root: string, name: string) {
  const path = join(root, name);
  return { type: "ESModule", path, contents: stripTypeScriptTypes(readFileSync(path, "utf8")) } as const;
}

/** Starts `server` on a free loopback port, which it returns. */
export async function listen(server: Server) {
  await new Promise<void>(resolve => server.listen(0, "127.0.0.1", resolve));
  const address = server.address();
  if (address === null || typeof address === "string") throw new Error("The server has no port");
  return address.port;
}
