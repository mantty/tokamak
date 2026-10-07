import { sharedBufferSourceBytes } from "../globals/conversions.mjs";
import { DOMException } from "../globals/dom-exception.mjs";
import { structuredClone } from "../globals/structured-clone.mjs";
import { TextEncoder } from "../streams/text.mjs";
import { CloseEvent, EventTarget, MessageEvent } from "../events/web.mjs";

// Connects two new sockets to each other, both open.
let pair;
// The host's end of `client`: `receive` and `close` deliver frames to the server socket,
// `take()` returns the frames the server sent, and `listen(notify)` runs `notify` whenever it sends one.
let nativeWebSocket;

export class WebSocket extends EventTarget {
  #readyState = 0;
  #binaryType = "blob";
  #accepted = false;
  #attachment;
  #peer;
  #outbox = [];
  #notify;

  get url() { return null; }
  get readyState() { return this.#readyState; }
  get protocol() { return ""; }
  get extensions() { return ""; }
  get binaryType() { return this.#binaryType; }
  set binaryType(value) {
    const next = String(value);
    if (next !== "blob" && next !== "arraybuffer") throw new SyntaxError("Invalid binaryType");
    this.#binaryType = next;
  }

  accept() { this.#accepted = true; this.#readyState = 1; }

  send(data) {
    if (!this.#accepted) throw new TypeError("WebSocket has not been accepted");
    const peer = this.#peer;
    if (peer === undefined || this.#readyState !== 1) throw new TypeError("WebSocket is not open");
    peer.#post({ type: "message", binary: typeof data !== "string", data: websocketData(data) });
  }

  close(code = 1000, reason = "") {
    const status = Number(code);
    if (!Number.isInteger(status) || !validCloseCode(status)) throw new DOMException("Invalid close code", "SyntaxError");
    const text = String(reason);
    if (new TextEncoder().encode(text).byteLength > 123) throw new DOMException("WebSocket close reason must not be longer than 123 bytes when UTF-8 encoded.", "SyntaxError");
    if (this.#readyState === 3 || this.#readyState === 2) return;
    this.#readyState = 2;
    this.#peer?.#post({ type: "close", code: status, reason: text });
  }

  serializeAttachment(value) {
    this.#attachment = structuredClone(value);
  }

  deserializeAttachment() {
    return this.#attachment;
  }

  #post(frame) {
    this.#outbox.push(frame);
    this.#notify?.();
  }

  #receiveMessage(data) {
    if (this.#readyState === 3) return;
    this.#readyState = 1;
    this.dispatchEvent(new MessageEvent("message", { data }));
  }

  #receiveClose(code, reason) {
    if (this.#readyState === 3) return;
    this.#readyState = 3;
    this.dispatchEvent(new CloseEvent("close", { code, reason, wasClean: true }));
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
        close: (code, reason) => server.#receiveClose(code, reason),
        take: () => client.#outbox.splice(0),
        listen: notify => { client.#notify = notify; },
      };
    };
  }
}

function validCloseCode(code) {
  return code === 1000 || code >= 1001 && code <= 1003 || code >= 1007 && code <= 1014 || code >= 3000 && code <= 4999;
}

export class WebSocketPair {
  constructor() {
    const client = new WebSocket();
    const server = new WebSocket();
    pair(client, server);
    this[0] = client;
    this[1] = server;
  }
}

function websocketData(data) {
  if (typeof data === "string") return data;
  const bytes = sharedBufferSourceBytes(data);
  if (!bytes) throw new TypeError("WebSocket data must be a string, ArrayBuffer, or ArrayBufferView");
  return bytes.slice().buffer;
}

export function installWebSocketGlobals() {
  globalThis.WebSocket ??= WebSocket;
  globalThis.WebSocketPair ??= WebSocketPair;
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
