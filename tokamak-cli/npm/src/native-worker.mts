import { X509Certificate } from "node:crypto";
import fs from "node:fs";
import type { ServerOptions as HttpsServerOptions } from "node:https";
import path from "node:path";

import type { PluginConfig } from "@cloudflare/vite-plugin";
import { normalizePath, type Plugin, type ViteDevServer } from "vite";

import { capabilitiesFor, type NativePlatform } from "@tokamakdev/plugin/capabilities";
import { openDevSocket, type DevSocket } from "./dev-socket.mjs";
import type { Config } from "./index.mjs";
import { findModule } from "./worker-contract.mjs";
import { createWorkerGraph, type WorkerGraph } from "./worker-graph.mjs";

export type TokamakOptions = Config & {
  /** Native worker entrypoint, relative to the Vite root; defaults to src/tokamak.ts or .js. */
  module?: string;
};

const CAPABILITIES = "/_tokamak/capabilities.js";
const MIDDLEWARE_MODE_ERROR =
  "Vite is running in middleware mode, so tokamak cannot find the development server's address; tok dev needs Vite's own dev server";

type AuxiliaryWorker = NonNullable<PluginConfig["auxiliaryWorkers"]>[number];

/** The native worker and its development relay. */
export function createNativeWorker(
  options: TokamakOptions,
  output: string,
  prepare?: (root: string, graph: WorkerGraph) => void | Promise<void>,
): { plugin: Plugin; graph(): WorkerGraph; auxiliaryWorker: AuxiliaryWorker } {
  const { module, ...config } = options;
  const token = process.env.TOKAMAK_SESSION_TOKEN;
  const platform = process.env.TOKAMAK_PLATFORM as NativePlatform | undefined;
  let graph: WorkerGraph;
  const plugin: Plugin = {
    name: "tokamak",
    enforce: "pre",
    async config(vite, environment) {
      const root = normalizePath(path.resolve(vite.root ?? process.cwd()));
      graph = createWorkerGraph(root, findModule(root, module), path.join(output, "worker"));
      graph.plugin.config.call(this, vite, environment);
      await prepare?.(root, graph);
      writeJson(output, "config.json", { root, config });
    },
    configEnvironment: {
      order: "post",
      handler(name, config) {
        return graph.plugin.configEnvironment.handler(name, config);
      },
    },
    buildStart(options) {
      return graph.plugin.buildStart.call(this, options);
    },
    hotUpdate(options) {
      return graph.plugin.hotUpdate.call(this, options);
    },
    resolveId(...args) {
      return graph.plugin.resolveId.call(this, ...args);
    },
    load(...args) {
      return graph.plugin.load.call(this, ...args);
    },
    configureServer: {
      order: "pre",
      async handler(server) {
        const devSocket = await serveNativeWorker(server, graph.names(), output, token);
        if (devSocket && token) graph.relay({ port: devSocket.port, token, platform });
        // A page loaded from this server, not the device, still gets the device's capabilities.
        server.middlewares.use(CAPABILITIES, (_request, response) => {
          const capabilities = capabilitiesFor(graph.availability(), platform);
          response.setHeader("content-type", "text/javascript; charset=utf-8");
          response.end(`export default ${JSON.stringify(capabilities)};\n`);
        });
      },
    },
  };
  return {
    plugin,
    graph: () => graph,
    // The graph exists once Vite's config is known, after Cloudflare's plugin is created.
    auxiliaryWorker: { viteEnvironment: { name: "tokamak" }, config: (worker, context) => graph.native(worker, context) },
  };
}

async function serveNativeWorker(
  server: ViteDevServer,
  names: ReturnType<WorkerGraph["names"]>,
  output: string,
  token: string | undefined,
): Promise<DevSocket | undefined> {
  const httpServer = server.httpServer;
  if (!httpServer) {
    if (!fs.existsSync(path.join(output, "server.json"))) {
      writeJson(output, "server.json", { error: MIDDLEWARE_MODE_ERROR });
    }
    return;
  }
  const socket = await openDevSocket(names.native, token);
  httpServer.once("close", () => void socket.close());
  httpServer.once("listening", () => {
    const urls = server.resolvedUrls;
    writeJson(output, "server.json", {
      url: urls?.local[0] ?? urls?.network[0],
      workerName: names.app,
      certificates: endEntityCertificates(server.config.server.https),
      socketPort: socket.port,
    });
  });
  return socket;
}

function endEntityCertificates(https: HttpsServerOptions | undefined): string | undefined {
  const cert = typeof https?.cert === "string" ? readFileIfExists(https.cert) : https?.cert;
  return cert && [cert].flat().map((chain) => new X509Certificate(chain).toString()).join("");
}

function readFileIfExists(file: string): string | Buffer {
  try {
    return fs.readFileSync(path.resolve(file));
  } catch {
    return file;
  }
}

/** Write reports atomically while tok watches for them. */
function writeJson(output: string, name: string, value: unknown): void {
  const file = path.join(output, name);
  fs.mkdirSync(output, { recursive: true });
  fs.writeFileSync(`${file}.tmp`, `${JSON.stringify(value, null, 2)}\n`);
  fs.renameSync(`${file}.tmp`, file);
}
