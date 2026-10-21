import path from "node:path";
import { fileURLToPath } from "node:url";
import { normalizePath, type Plugin } from "vite";

import { findModule } from "./worker-contract.mjs";
import { WorkerTypes, type BindingConfiguration } from "./worker-types.mjs";

const MODULE = "virtual:tokamak/client";
/** The Tokamak worker's page module of each plugin operation's support on the app's platform. */
const CAPABILITIES = "/_tokamak/capabilities.js";
const TRANSPORT = normalizePath(fileURLToPath(new URL("./rpc/client.mjs", import.meta.resolve("@tokamakdev/plugin/events"))));

/** The app's public methods, without its native implementation. */
export function createClientPlugin(module: string | undefined, native: boolean, shared?: () => WorkerTypes): Plugin & {
  configure(config: BindingConfiguration): void;
} {
  let root: string;
  /** Whether `id` is a generated client: the app's, a named entrypoint's or a plugin's. */
  const isClient = (id: string) => id === MODULE || id.startsWith(`${MODULE}/`) ||
    contracts.definitions.some(plugin => plugin.client === id);
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
      if (isClient(id)) return `\0${id}`;
    },
    async load(id) {
      const name = id.slice(1);
      if (!id.startsWith("\0") || !isClient(name)) return;
      await contracts.resolve(source => this.resolve(source, undefined, { skipSelf: true }));
      const service = contracts.services().find(service => service.client === name);
      if (!service) throw new Error(`${name} names no Tokamak worker entrypoint.`);
      clients.add(id);
      files = [...contracts.native.files, ...contracts.plugins.flatMap(plugin => plugin.files)]
        .filter(file => file !== normalizePath(path.join(root, "tokamak-env.d.ts")));
      for (const file of files) this.addWatchFile(file);
      const methods = JSON.stringify(service.contract.methods);
      const route = JSON.stringify(service.route);
      const plugin = service.plugin;
      if (native && plugin) return `import { createClient } from ${JSON.stringify(TRANSPORT)};
import capabilities from ${JSON.stringify(CAPABILITIES)};
export default createClient(${methods}, ${route}, capabilities[${JSON.stringify(plugin.definition.id)}]);`;
      // The app's own operations run wherever its Tokamak worker does.
      if (native) return `import { createClient } from ${JSON.stringify(TRANSPORT)};
export default createClient(${methods}, ${route}, Object.fromEntries(${methods}.map(method => [method, true])));`;
      const browser = plugin?.definition.web;
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
      const loaded = [...clients].map(id => this.environment.moduleGraph.getModuleById(id)).filter(client => client !== undefined);
      for (const client of loaded) this.environment.moduleGraph.invalidateModule(client);
      if (loaded.length) this.environment.hot.send({ type: "full-reload" });
    },
  };
}
