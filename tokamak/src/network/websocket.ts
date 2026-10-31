import type { NativeSocket, OutboxFrame } from "tokamak:host";
import { sharedBufferSourceBytes } from "../globals/conversions.ts";
import { DOMException } from "../globals/dom-exception.ts";
import { structuredClone } from "../globals/structured-clone.ts";
import { TextEncoder } from "../streams/text.ts";
import { CloseEvent, EventTarget, MessageEvent } from "../events/web.ts";

// Connects two new sockets to each other, both open.
let pair: (client: WebSocket, server: WebSocket) => void;
// The host's end of `client`: `receive` and `close` deliver frames to the server socket,
// `disconnect()` tells it the connection dropped without a close frame, `take()` returns
// the frames the server sent, and `listen(notifier)` calls `notifier.notify()` whenever it sends one.
let nativeWebSocket: (client: WebSocket) => NativeSocket;

export class WebSocket extends EventTarget {
  #readyState = 0;
  #binaryType: "blob" | "arraybuffer" = "blob";
  #accepted = false;
  #attachment: unknown;
  #peer: WebSocket | undefined;
  #outbox: OutboxFrame[] = [];
  #notify: (() => void) | undefined;

  get url() { return null; }
  get readyState() { return this.#readyState; }
  get protocol() { return ""; }
  get extensions() { return ""; }
  get binaryType() { return this.#binaryType; }
  set binaryType(value: unknown) {
    const next = String(value);
    if (next !== "blob" && next !== "arraybuffer") throw new SyntaxError("Invalid binaryType");
    this.#binaryType = next;
  }

  accept() { this.#accepted = true; this.#readyState = 1; }

  send(data: unknown): void {
    if (!this.#accepted) throw new TypeError("WebSocket has not been accepted");
    const peer = this.#peer;
    if (peer === undefined || this.#readyState !== 1) throw new TypeError("WebSocket is not open");
    peer.#post(messageFrame(data));
  }

  close(code: unknown = 1000, reason: unknown = ""): void {
    const status = Number(code);
    if (!Number.isInteger(status) || !validCloseCode(status)) throw new DOMException("Invalid close code", "SyntaxError");
    const text = String(reason);
    if (new TextEncoder().encode(text).byteLength > 123) throw new DOMException("WebSocket close reason must not be longer than 123 bytes when UTF-8 encoded.", "SyntaxError");
    if (this.#readyState === 3 || this.#readyState === 2) return;
    this.#readyState = 2;
    if (this.#peer) this.#peer.#post({ type: "close", code: status, reason: text });
  }

  serializeAttachment(value: unknown): void {
    this.#attachment = structuredClone(value);
  }

  deserializeAttachment(): unknown {
    return this.#attachment;
  }

  #post(frame: OutboxFrame) {
    this.#outbox.push(frame);
    this.#notify?.();
  }

  #receiveMessage(data: string | ArrayBuffer) {
    if (this.#readyState === 3) return;
    this.dispatchEvent(new MessageEvent("message", { data }));
  }

  #receiveClose(code: number, reason: string, wasClean: boolean) {
    if (this.#readyState === 3) return;
    this.#readyState = 3;
    this.dispatchEvent(new CloseEvent("close", { code, reason, wasClean }));
  }

  static {
    pair = (client, server) => {
      client.#readyState = 1;
      server.#readyState = 1;
      client.#peer = server;
      server.#peer = client;
    };
    nativeWebSocket = client => {
      const server = client.#peer;
      if (server === undefined) throw new TypeError("The WebSocket has no peer");
      return {
        receive: data => server.#receiveMessage(data),
        close: (code, reason) => server.#receiveClose(code, reason, true),
        disconnect: () => server.#receiveClose(1006, "WebSocket disconnected without sending Close frame.", false),
        take: () => client.#outbox.splice(0),
        listen: notifier => { client.#notify = () => { notifier.notify(); }; },
      };
    };
  }
}

function validCloseCode(code: number) {
  return code === 1000 || code >= 1001 && code <= 1003 || code >= 1007 && code <= 1014 || code >= 3000 && code <= 4999;
}

export class WebSocketPair {
  declare 0: WebSocket;
  declare 1: WebSocket;

  constructor() {
    const client = new WebSocket();
    const server = new WebSocket();
    pair(client, server);
    this[0] = client;
    this[1] = server;
  }
}

// The message frame that sends `data`.
function messageFrame(data: unknown): OutboxFrame {
  if (typeof data === "string") return { type: "message", binary: false, data };
  const bytes = sharedBufferSourceBytes(data);
  if (!bytes) throw new TypeError("WebSocket data must be a string, ArrayBuffer, or ArrayBufferView");
  return { type: "message", binary: true, data: bytes.slice().buffer };
}

for (const [name, value] of Object.entries({
  CONNECTING: 0,
  OPEN: 1,
  CLOSING: 2,
  CLOSED: 3,
  READY_STATE_CONNECTING: 0,
  READY_STATE_OPEN: 1,
  READY_STATE_CLOSING: 2,
  READY_STATE_CLOSED: 3,
})) {
  Object.defineProperty(WebSocket, name, { configurable: true, enumerable: true, value, writable: false });
  Object.defineProperty(WebSocket.prototype, name, { configurable: true, enumerable: true, value, writable: false });
}

export { nativeWebSocket };
