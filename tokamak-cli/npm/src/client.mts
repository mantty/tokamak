import path from "node:path";
import { fileURLToPath } from "node:url";
import { normalizePath, type Plugin } from "vite";

import { findModule } from "./worker-contract.mjs";
import { WorkerTypes, type BindingConfiguration } from "./worker-types.mjs";

const MODULE = "virtual:tokamak/client";
const RESOLVED = `\0${MODULE}`;
const TRANSPORT = normalizePath(fileURLToPath(new URL("./rpc/client.mjs", import.meta.resolve("@tokamakdev/plugin/events"))));

/** The app's public methods, without its native implementation. */
export function createClientPlugin(module: string | undefined, native: boolean, shared?: () => WorkerTypes): Plugin & {
  configure(config: BindingConfiguration): void;
} {
  let root: string;
  let contracts: WorkerTypes;
  let files: string[] = [];
  return {
    name: "tokamak:client",
    config(config) {
      root = path.resolve(config.root ?? process.cwd());
      contracts = shared?.() ?? new WorkerTypes(root, findModule(root, module));
    },
    configure(config) { contracts.configure(config); },
    resolveId(id) { if (id === MODULE) return RESOLVED; },
    async load(id) {
      if (id !== RESOLVED) return;
      await contracts.resolve(source => this.resolve(source, undefined, { skipSelf: true }));
      const contract = contracts.native;
      files = contract.files.filter(file => file !== normalizePath(path.join(root, "tokamak-env.d.ts")));
      for (const file of files) this.addWatchFile(file);
      const methods = JSON.stringify(contract.methods);
      if (native) return `import { createClient } from ${JSON.stringify(TRANSPORT)};
export default createClient(${methods}, "/_tokamak/app");`;
      return `async function unavailable() { throw new DOMException("This operation requires a native app", "NotSupportedError"); }
export default Object.fromEntries(${methods}.map(method => [method, unavailable]));`;
    },
    hotUpdate({ file, timestamp }) {
      if (!files.includes(file)) return;
      contracts.refresh({ file, timestamp });
      const client = this.environment.moduleGraph.getModuleById(RESOLVED);
      if (!client) return;
      this.environment.moduleGraph.invalidateModule(client);
      this.environment.hot.send({ type: "full-reload" });
    },
  };
}
