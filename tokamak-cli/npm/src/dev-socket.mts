import http from "node:http";
import type { AddressInfo } from "node:net";
import { Readable, type Duplex } from "node:stream";
import { pipeline } from "node:stream/promises";
import type { ReadableStream } from "node:stream/web";

import { WebSocketServer, type WebSocket } from "ws";
import { decodeMessage, encodeMessage, type NativeMessage } from "@tokamakdev/plugin/native-protocol";
import { DevWorker, type WorkerSocket } from "./dev-worker.mjs";

/** The header that carries the device's session token, which `tok dev`'s relay forwards. */
export const SESSION_HEADER = "x-tokamak-session";

/** How long a plugin call waits for the device to connect. */
const CONNECT_TIMEOUT = 10_000;

/** The native endpoint's opaque result and completion status. */
export interface PluginResult {
  payload: Uint8Array;
  failed: boolean;
  done: boolean;
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
    if (request.url?.startsWith("/_tokamak/")) {
      void forwardWorker(worker, request, response);
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
    if (request.url?.startsWith("/_tokamak/")) {
      void upgradeWorker(worker, request, socket, head, sockets);
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

async function forwardWorker(worker: DevWorker, request: http.IncomingMessage, response: http.ServerResponse): Promise<void> {
  const abort = new AbortController();
  response.once("close", () => abort.abort());
  try {
    const result = await worker.fetch(request, abort.signal);
    response.writeHead(result.status, [...result.headers].flat());
    if (result.body) await pipeline(Readable.fromWeb(result.body as ReadableStream<Uint8Array>), response);
    else response.end();
  } catch (error) {
    if (response.headersSent) response.destroy(error instanceof Error ? error : new Error(String(error)));
    else response.writeHead(502).end(String(error));
  }
}

async function upgradeWorker(
  worker: DevWorker, request: http.IncomingMessage, socket: Duplex, head: Buffer, sockets: WebSocketServer,
): Promise<void> {
  const abort = new AbortController();
  let pending: WorkerSocket | undefined;
  const release = () => {
    const pair = pending;
    pending = undefined;
    if (!pair) return;
    pair.accept();
    pair.close(1000, "Caller disconnected");
  };
  socket.once("close", () => { abort.abort(); release(); });
  socket.once("error", () => socket.destroy());
  try {
    const result = await worker.fetch(request, abort.signal);
    const pair = result.webSocket;
    if (!pair) {
      socket.end(`HTTP/1.1 ${result.status} ${result.statusText}\r\nConnection: close\r\n\r\n`);
      return;
    }
    pending = pair;
    if (socket.destroyed) {
      release();
      return;
    }
    sockets.handleUpgrade(request, socket, head, websocket => {
      void worker.couple(websocket, pair).then(
        () => { pending = undefined; },
        () => { release(); websocket.terminate(); },
      );
    });
  } catch {
    release();
    socket.end("HTTP/1.1 502 Bad Gateway\r\nConnection: close\r\n\r\n");
  }
}

function serve(websocket: WebSocket, worker: DevWorker, device: Device): void {
  device.connect(websocket);
  // The device opens another dev WebSocket after this one fails.
  websocket.on("error", () => websocket.terminate());
  websocket.on("message", (data) => {
    let message: NativeMessage;
    try {
      const bytes = Array.isArray(data) ? Buffer.concat(data) : new Uint8Array(data as ArrayBuffer);
      message = decodeMessage(bytes);
    } catch {
      websocket.close(1002, "Invalid native message");
      return;
    }
    const { header, payload } = message;
    if (header.type === "result" && header.id !== undefined) {
      device.receive(header.id, { payload, failed: header.failed === true, done: header.done === true });
      return;
    }
    if (header.type !== "event" || header.id === undefined || !header.name) return;
    const send = (type: string, value: Uint8Array) => {
      if (websocket.readyState === websocket.OPEN) websocket.send(encodeMessage({ type, id: header.id }, value));
    };
    worker.dispatch(header.name, payload).then(
      (reply) => send("reply", reply),
      (error: unknown) => send("error", encoder.encode(String(error))),
    );
  });
}

const encoder = new TextEncoder();

/** Relays opaque endpoint payloads; closing the response cancels the native request. */
async function forward(device: Device, request: http.IncomingMessage, response: http.ServerResponse): Promise<void> {
  let call: NativeMessage;
  try {
    const chunks: Buffer[] = [];
    for await (const chunk of request) chunks.push(chunk as Buffer);
    call = decodeMessage(Buffer.concat(chunks));
    if (!call.header.plugin || !call.header.method) throw new TypeError("Missing native endpoint");
  } catch {
    response.writeHead(400).end();
    return;
  }
  response.writeHead(200, { "content-type": "application/octet-stream" });
  const subscribe = request.url === "/subscribe";
  if (subscribe) response.flushHeaders();
  const cancel = device.send(subscribe ? "subscribe" : "call", call, ({ payload, failed, done }) => {
    const frame = encodeMessage({ type: "result", failed, done }, payload);
    if (!subscribe || done) response.end(frame);
    else {
      response.write(frame);
      if (response.writableLength > 1024 * 1024) response.destroy(new Error("Native subscription consumer is too slow"));
    }
  });
  response.once("close", () => {
    if (!response.writableEnded) cancel();
  });
}

function failure(name: string, message: string): PluginResult {
  return { payload: encoder.encode(JSON.stringify({ name, message })), failed: true, done: true };
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
   * and passes each of its results to `receive`. Returns the function that cancels the request.
   */
  send(type: "call" | "subscribe", request: NativeMessage, receive: (result: PluginResult) => void): () => void {
    const id = this.#nextId++;
    let cancelled = false;
    void this.#connection().then((socket) => {
      if (cancelled) return;
      if (!socket) {
        receive(failure("NetworkError", "The device is not connected"));
        return;
      }
      this.#pending.set(id, { type, socket, receive });
      socket.send(encodeMessage({ ...request.header, type, id }, request.payload));
    });
    return () => {
      cancelled = true;
      const pending = this.#pending.get(id);
      if (!pending) return;
      this.#pending.delete(id);
      pending.socket.send(encodeMessage({ type: "cancel", id }));
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
