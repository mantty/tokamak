import http from "node:http";
import type { AddressInfo } from "node:net";
import { createRequire } from "node:module";
import { pathToFileURL } from "node:url";

import { WebSocketServer, type WebSocket } from "ws";

/** The header that carries the device's session token, which `tok dev`'s relay forwards. */
export const SESSION_HEADER = "x-tokamak-session";

/** How long a plugin call waits for the device to connect. */
const CONNECT_TIMEOUT = 10_000;

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

/** A message the device sends: an event, or a plugin's result for a call or subscription. */
type DeviceMessage =
  | { type: "event"; id: number; name: string; event: unknown }
  | { type: "result"; id: number; result: PluginResult };

/** A plugin's result, as the page's native transport receives it. */
export interface PluginResult {
  value?: unknown;
  error?: { name: string; message: string };
  done: boolean;
}

/** A call or subscription the development Worker makes on a plugin. */
interface PluginRequest {
  plugin: string;
  method: string;
  arguments: unknown;
}

/** The loopback server the device's dev WebSocket reaches through `tok dev`'s relay. */
export interface DevSocket {
  readonly port: number;
  close(): Promise<void>;
}

/**
 * Listens on a loopback port, with `token`, for the device's dev WebSocket and the development
 * Worker's plugin calls. Delivers each event the device sends to `TokamakEvents` in the
 * development Worker `workerName`, and passes each plugin call the Worker posts to `/call` or
 * `/subscribe` to the device.
 */
export async function openDevSocket(workerName: string | undefined, token: string | undefined): Promise<DevSocket> {
  const worker = new DevWorker(workerName);
  const device = new Device();
  const server = http.createServer((request, response) => {
    if (!token || request.headers[SESSION_HEADER] !== token) {
      response.writeHead(401).end();
      return;
    }
    if (request.method === "POST" && (request.url === "/call" || request.url === "/subscribe")) {
      void forward(device, request, response);
      return;
    }
    response.writeHead(request.method === "GET" ? 426 : 404).end();
  });
  const sockets = new WebSocketServer({ noServer: true });
  server.on("upgrade", (request, socket, head) => {
    if (!token || request.headers[SESSION_HEADER] !== token) {
      socket.end("HTTP/1.1 401 Unauthorized\r\nConnection: close\r\n\r\n");
      return;
    }
    sockets.handleUpgrade(request, socket, head, (websocket) => serve(websocket, worker, device));
  });
  await new Promise<void>((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolve);
  });
  return {
    port: (server.address() as AddressInfo).port,
    async close() {
      for (const websocket of sockets.clients) websocket.terminate();
      server.closeAllConnections();
      server.close();
      await worker.dispose();
    },
  };
}

function serve(websocket: WebSocket, worker: DevWorker, device: Device): void {
  device.connect(websocket);
  // The device opens another dev WebSocket after this one fails.
  websocket.on("error", () => websocket.terminate());
  websocket.on("message", (data) => {
    const message = parseMessage(String(data));
    if (message?.type === "result") {
      device.receive(message.id, message.result);
      return;
    }
    if (message?.type !== "event") return;
    worker.dispatch(message.name, message.event).then(
      (reply) => websocket.send(JSON.stringify({ type: "reply", id: message.id, reply })),
      (error: unknown) => websocket.send(JSON.stringify({ type: "error", id: message.id, message: String(error) })),
    );
  });
}

/**
 * Passes the plugin call `request` carries to the device. A call's response is its result; a
 * subscription's streams each result as a line of JSON until one is done, and closing it ends the
 * subscription.
 */
async function forward(device: Device, request: http.IncomingMessage, response: http.ServerResponse): Promise<void> {
  let call: PluginRequest;
  try {
    call = JSON.parse(await readBody(request)) as PluginRequest;
  } catch {
    response.writeHead(400).end();
    return;
  }
  response.writeHead(200, { "content-type": "application/json" });
  if (request.url === "/call") {
    response.end(JSON.stringify(await new Promise<PluginResult>((resolve) => device.send("call", call, resolve))));
    return;
  }
  response.flushHeaders();
  const cancel = device.send("subscribe", call, (result) => {
    response.write(`${JSON.stringify(result)}\n`);
    if (result.done) response.end();
  });
  response.once("close", () => {
    if (!response.writableEnded) cancel();
  });
}

function readBody(request: http.IncomingMessage): Promise<string> {
  return new Promise((resolve, reject) => {
    const chunks: Buffer[] = [];
    request.on("data", (chunk: Buffer) => chunks.push(chunk));
    request.once("end", () => resolve(Buffer.concat(chunks).toString()));
    request.once("error", reject);
  });
}

function parseMessage(text: string): DeviceMessage | undefined {
  try {
    return JSON.parse(text) as DeviceMessage;
  } catch {
    return undefined;
  }
}

/** A failed result, which ends a subscription. */
function failure(name: string, message: string): PluginResult {
  return { error: { name, message }, done: true };
}

/** The device, through its latest dev WebSocket, and the plugin calls waiting for its results. */
class Device {
  #socket: WebSocket | undefined;
  #connected: (() => void)[] = [];
  #nextId = 1;
  #pending = new Map<number, { type: "call" | "subscribe"; socket: WebSocket; receive(result: PluginResult): void }>();

  connect(socket: WebSocket): void {
    this.#socket = socket;
    for (const connected of this.#connected.splice(0)) connected();
    socket.once("close", () => {
      if (this.#socket === socket) this.#socket = undefined;
      for (const [id, pending] of this.#pending) {
        if (pending.socket !== socket) continue;
        this.#pending.delete(id);
        pending.receive(failure("NetworkError", "The device disconnected"));
      }
    });
  }

  receive(id: number, result: PluginResult): void {
    const pending = this.#pending.get(id);
    if (!pending) return;
    if (result.done || pending.type === "call") this.#pending.delete(id);
    pending.receive(result);
  }

  /**
   * Sends a call or subscription once the device is connected, waiting up to 10 seconds for it,
   * and passes each of its results to `receive`. Returns the function that unsubscribes.
   */
  send(type: "call" | "subscribe", request: PluginRequest, receive: (result: PluginResult) => void): () => void {
    const id = this.#nextId++;
    let cancelled = false;
    void this.#connection().then((socket) => {
      if (cancelled) return;
      if (!socket) {
        receive(failure("NetworkError", "The device is not connected"));
        return;
      }
      this.#pending.set(id, { type, socket, receive });
      socket.send(JSON.stringify({ type, id, ...request }));
    });
    return () => {
      cancelled = true;
      const pending = this.#pending.get(id);
      if (!pending) return;
      this.#pending.delete(id);
      pending.socket.send(JSON.stringify({ type: "unsubscribe", id }));
    };
  }

  /** The open dev WebSocket, once the device connects, or undefined after 10 seconds without. */
  #connection(): Promise<WebSocket | undefined> {
    if (this.#socket) return Promise.resolve(this.#socket);
    return new Promise((resolve) => {
      const connected = () => {
        clearTimeout(timer);
        resolve(this.#socket);
      };
      const timer = setTimeout(() => {
        this.#connected.splice(this.#connected.indexOf(connected), 1);
        resolve(undefined);
      }, CONNECT_TIMEOUT);
      this.#connected.push(connected);
    });
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
