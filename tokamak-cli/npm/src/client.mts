import path from "node:path";
import { fileURLToPath } from "node:url";
import { normalizePath, type Plugin } from "vite";

import { findModule } from "./worker-contract.mjs";
import { WorkerTypes, type BindingConfiguration } from "./worker-types.mjs";

const MODULE = "virtual:tokamak/client";
/** The Tokamak worker's page module of each plugin operation's support on the app's platform. */
const CAPABILITIES = "/_tokamak/capabilities.js";
const RESOLVED = `\0${MODULE}`;
const TRANSPORT = normalizePath(fileURLToPath(new URL("./rpc/client.mjs", import.meta.resolve("@tokamakdev/plugin/events"))));

/** The app's public methods, without its native implementation. */
export function createClientPlugin(module: string | undefined, native: boolean, shared?: () => WorkerTypes): Plugin & {
  configure(config: BindingConfiguration): void;
} {
  let root: string;
  let contracts: WorkerTypes;
  let files: string[] = [];
  const clients = new Set<string>();
  return {
    name: "tokamak:client",
    enforce: "pre",
    config(config) {
      clients.clear();
      files = [];
      root = path.resolve(config.root ?? process.cwd());
      contracts = shared?.() ?? new WorkerTypes(root, findModule(root, module));
    },
    configure(config) { contracts.configure(config); },
    buildStart() {
      for (const file of contracts.metadata) this.addWatchFile(file);
    },
    resolveId(id) {
      if (id === CAPABILITIES) return { id, external: true };
      if (id === MODULE) return RESOLVED;
      if (contracts.definitions.some(plugin => plugin.client === id)) return `\0${id}`;
    },
    async load(id) {
      const definition = contracts.definitions.find(plugin => `\0${plugin.client}` === id);
      if (id !== RESOLVED && !definition) return;
      await contracts.resolve(source => this.resolve(source, undefined, { skipSelf: true }));
      const plugin = definition && contracts.plugins.find(plugin => plugin.definition.id === definition.id)!;
      const contract = plugin ?? contracts.native;
      clients.add(id);
      files = [...contracts.native.files, ...contracts.plugins.flatMap(plugin => plugin.files)]
        .filter(file => file !== normalizePath(path.join(root, "tokamak-env.d.ts")));
      for (const file of files) this.addWatchFile(file);
      const methods = JSON.stringify(contract.methods);
      if (native && definition) return `import { createClient } from ${JSON.stringify(TRANSPORT)};
import capabilities from ${JSON.stringify(CAPABILITIES)};
export default createClient(${methods}, ${JSON.stringify(`/_tokamak/plugin/${definition.id}`)}, capabilities[${JSON.stringify(definition.id)}]);`;
      // The app's own operations run wherever its Tokamak worker does.
      if (native) return `import { createClient } from ${JSON.stringify(TRANSPORT)};
export default createClient(${methods}, "/_tokamak/app", Object.fromEntries(${methods}.map(method => [method, true])));`;
      const browser = definition?.web;
      const fallback = browser
        ? `import { ${JSON.stringify(browser.export)} as browser } from ${JSON.stringify(browser.module)};`
        : "const browser = {};";
      const webMethods = JSON.stringify(plugin?.webMethods ?? []);
      return `${fallback}
async function unavailable() { throw new DOMException("This operation requires a native app", "NotSupportedError"); }
const supported = new Set(${webMethods});
const client = Object.fromEntries(${methods}.map(method => [method, supported.has(method) ? (...args) => Promise.resolve().then(() => browser[method](...args)) : unavailable]));
export default Object.assign(client, { capabilities: Object.freeze(Object.fromEntries(${methods}.map(method => [method, supported.has(method)]))) });`;
    },
    hotUpdate({ file, timestamp, server }) {
      if (contracts.metadata.includes(file)) {
        if (this.environment.name === "client") {
          try {
            contracts.discover();
          } catch (error) {
            server.config.logger.error(String(error));
            throw error;
          }
          void server.restart().catch(error => server.config.logger.error(String(error)));
        }
        return [];
      }
      if (!files.includes(file)) return;
      contracts.refresh({ file, timestamp });
      for (const id of clients) {
        const client = this.environment.moduleGraph.getModuleById(id);
        if (!client) continue;
        this.environment.moduleGraph.invalidateModule(client);
        this.environment.hot.send({ type: "full-reload" });
      }
    },
  };
}
