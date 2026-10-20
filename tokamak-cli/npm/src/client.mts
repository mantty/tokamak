import path from "node:path";
import { fileURLToPath } from "node:url";
import { normalizePath, type Plugin } from "vite";

import { findModule, readWorkerContract } from "./worker-contract.mjs";
import { serviceTypes, writeEnvironmentTypes } from "./service-types.mjs";

const MODULE = "virtual:tokamak/client";
const RESOLVED = `\0${MODULE}`;
const TRANSPORT = normalizePath(fileURLToPath(new URL("./rpc/client.mjs", import.meta.resolve("@tokamakdev/plugin/events"))));

/** The app's public methods, without its native implementation. */
export function createClientPlugin(module: string | undefined, native: boolean): Plugin {
  let root: string;
  let files: string[] = [];
  return {
    name: "tokamak:client",
    configResolved(config) { root = config.root; },
    resolveId(id) { if (id === MODULE) return RESOLVED; },
    load(id) {
      if (id !== RESOLVED) return;
      const entry = findModule(root, module);
      const contract = entry ? readWorkerContract(entry, root) : { methods: [], files: [], declarations: serviceTypes("{}", []) };
      files = contract.files.filter(file => file !== normalizePath(path.join(root, "tokamak-env.d.ts")));
      for (const file of files) this.addWatchFile(file);
      writeEnvironmentTypes(root, contract.declarations);
      const methods = JSON.stringify(contract.methods);
      if (native) return `import { createClient } from ${JSON.stringify(TRANSPORT)};
export default createClient(${methods}, "/_tokamak/app");`;
      return `async function unavailable() { throw new DOMException("This operation requires a native app", "NotSupportedError"); }
export default Object.fromEntries(${methods}.map(method => [method, unavailable]));`;
    },
    hotUpdate({ file }) {
      if (!files.includes(file)) return;
      const client = this.environment.moduleGraph.getModuleById(RESOLVED);
      if (!client) return;
      this.environment.moduleGraph.invalidateModule(client);
      this.environment.hot.send({ type: "full-reload" });
    },
  };
}
