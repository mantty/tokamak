import http from "node:http";
import type { AddressInfo } from "node:net";
import { createRequire } from "node:module";
import { pathToFileURL } from "node:url";

import { WebSocketServer, type WebSocket } from "ws";

/** The header `tok dev`'s relay forwards with the device's session token. */
const SESSION_HEADER = "x-tokamak-session";

/** Calls `TokamakEvents` in the Worker its `TOKAMAK` service binding names, through the dev registry. */
const CALLER = `export default {
  async fetch(request, env) {
    const { name, event } = await request.json();
    try {
      const { reply } = await env.TOKAMAK.dispatch(name, event);
      return Response.json({ reply: reply ?? null });
    } catch (error) {
      return new Response(error?.stack ?? String(error), { status: 500 });
    }
  },
};`;

/** The parts of Miniflare the dev socket uses. */
interface Miniflare {
  readonly ready: Promise<unknown>;
  dispatchFetch(url: string, init: { method: string; body: string }): Promise<Response>;
  dispose(): Promise<void>;
}

interface MiniflareModule {
  Miniflare: new (options: unknown) => Miniflare;
  convertV4MiniflareOptions(options: unknown): unknown;
  getDefaultDevRegistryPath(): string;
}

/** A message the device sends. */
interface EventMessage {
  type: "event";
  id: number;
  name: string;
  event: unknown;
}

/** The loopback server the device's dev WebSocket reaches through `tok dev`'s relay. */
export interface DevSocket {
  readonly port: number;
  close(): Promise<void>;
}

/**
 * Listens on a loopback port for dev WebSockets that carry `token`, and delivers each event they
 * carry to `TokamakEvents` in the development Worker `workerName`, replying with its reply.
 */
export async function openDevSocket(workerName: string | undefined, token: string | undefined): Promise<DevSocket> {
  const worker = new DevWorker(workerName);
  const server = http.createServer((_request, response) => response.writeHead(426).end());
  const sockets = new WebSocketServer({ noServer: true });
  server.on("upgrade", (request, socket, head) => {
    if (!token || request.headers[SESSION_HEADER] !== token) {
      socket.end("HTTP/1.1 401 Unauthorized\r\nConnection: close\r\n\r\n");
      return;
    }
    sockets.handleUpgrade(request, socket, head, (websocket) => serve(websocket, worker));
  });
  await new Promise<void>((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolve);
  });
  return {
    port: (server.address() as AddressInfo).port,
    async close() {
      for (const websocket of sockets.clients) websocket.terminate();
      server.close();
      await worker.dispose();
    },
  };
}

function serve(websocket: WebSocket, worker: DevWorker): void {
  // The device opens another dev WebSocket after this one fails.
  websocket.on("error", () => websocket.terminate());
  websocket.on("message", (data) => {
    const message = parseMessage(String(data));
    if (message?.type !== "event") return;
    worker.dispatch(message.name, message.event).then(
      (reply) => websocket.send(JSON.stringify({ type: "reply", id: message.id, reply })),
      (error: unknown) => websocket.send(JSON.stringify({ type: "error", id: message.id, message: String(error) })),
    );
  });
}

function parseMessage(text: string): EventMessage | undefined {
  try {
    return JSON.parse(text) as EventMessage;
  } catch {
    return undefined;
  }
}

/** The development Worker's `TokamakEvents`, called through a Miniflare session started on first use. */
class DevWorker {
  #miniflare: Promise<Miniflare> | undefined;

  constructor(private readonly name: string | undefined) {}

  async dispatch(name: string, event: unknown): Promise<unknown> {
    this.#miniflare ??= this.#start().catch((error: unknown) => {
      this.#miniflare = undefined;
      throw error;
    });
    const miniflare = await this.#miniflare;
    const response = await miniflare.dispatchFetch("http://tokamak/", {
      method: "POST",
      body: JSON.stringify({ name, event }),
    });
    if (!response.ok) throw new Error(await response.text());
    return ((await response.json()) as { reply: unknown }).reply;
  }

  async dispose(): Promise<void> {
    await (await this.#miniflare?.catch(() => undefined))?.dispose();
  }

  /** A Miniflare session from the Miniflare that Cloudflare's plugin uses. */
  async #start(): Promise<Miniflare> {
    if (!this.name) throw new Error("the Wrangler configuration names no Worker to deliver events to");
    const require = createRequire(import.meta.resolve("@cloudflare/vite-plugin"));
    const module = (await import(pathToFileURL(require.resolve("miniflare")).href)) as MiniflareModule;
    const miniflare = new module.Miniflare(
      module.convertV4MiniflareOptions({
        modules: true,
        script: CALLER,
        compatibilityDate: "2025-01-01",
        serviceBindings: { TOKAMAK: { name: this.name, entrypoint: "TokamakEvents" } },
        unsafeDevRegistryPath: module.getDefaultDevRegistryPath(),
      }),
    );
    await miniflare.ready;
    return miniflare;
  }
}
