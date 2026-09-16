import { CloseEvent, EventTarget, MessageEvent } from "../events/events.mjs";

export class WebSocket extends EventTarget {
  constructor() {
    super();
    this.__url = null;
    this.__readyState = 0;
    this.__protocol = "";
    this.__extensions = "";
    this.__binaryType = "blob";
    this.__accepted = false;
    this.__attachment = undefined;
    this.__tokamak_peer = undefined;
    this.__tokamak_outbox = [];
    this.__tokamak_receive = (data, binary) => {
      if (this.__readyState === 3) return;
      this.__readyState = 1;
      this.dispatchEvent(new MessageEvent("message", {
        data: binary && data instanceof ArrayBuffer ? data : data,
      }));
    };
    this.__tokamak_close = (code, reason) => {
      if (this.__readyState === 3) return;
      this.__readyState = 3;
      this.dispatchEvent(new CloseEvent("close", { code, reason, wasClean: true }));
    };
  }

  get url() { return this.__url; }
  get readyState() { return this.__readyState; }
  get protocol() { return this.__protocol; }
  get extensions() { return this.__extensions; }
  get binaryType() { return this.__binaryType; }
  set binaryType(value) {
    const next = String(value);
    if (next !== "blob" && next !== "arraybuffer") throw new SyntaxError("Invalid binaryType");
    this.__binaryType = next;
  }

  accept() { this.__accepted = true; this.__readyState = 1; }

  send(data) {
    if (!this.__accepted) throw new TypeError("WebSocket has not been accepted");
    const peer = this.__tokamak_peer;
    if (peer === undefined || this.__readyState !== 1) throw new TypeError("WebSocket is not open");
    peer.__tokamak_outbox.push({ type: "message", binary: typeof data !== "string", data: websocketData(data) });
    peer.__tokamak_notify?.();
  }

  close(code = 1000, reason = "") {
    const status = Number(code);
    if (!Number.isInteger(status) || !validCloseCode(status)) throw new DOMException("Invalid close code", "SyntaxError");
    const text = String(reason);
    if (new TextEncoder().encode(text).byteLength > 123) throw new DOMException("WebSocket close reason must not be longer than 123 bytes when UTF-8 encoded.", "SyntaxError");
    if (this.__readyState === 3 || this.__readyState === 2) return;
    this.__readyState = 2;
    const peer = this.__tokamak_peer;
    if (peer !== undefined) peer.__tokamak_outbox.push({ type: "close", code: status, reason: text });
    peer?.__tokamak_notify?.();
  }

  serializeAttachment(value) {
    this.__attachment = globalThis.structuredClone?.(value);
  }

  deserializeAttachment() {
    return this.__attachment;
  }
}

function validCloseCode(code) {
  return code === 1000 || code >= 1001 && code <= 1003 || code >= 1007 && code <= 1014 || code >= 3000 && code <= 4999;
}

export class WebSocketPair {
  constructor() {
    const client = new WebSocket();
    const server = new WebSocket();
    client.__readyState = 1;
    server.__readyState = 1;
    client.__tokamak_peer = server;
    server.__tokamak_peer = client;
    this[0] = client;
    this[1] = server;
  }
}

function websocketData(data) {
  if (typeof data === "string") return data;
  if (data instanceof ArrayBuffer) return data.slice(0);
  if (ArrayBuffer.isView(data)) return data.buffer.slice(data.byteOffset, data.byteOffset + data.byteLength);
  throw new TypeError("WebSocket data must be a string, ArrayBuffer, or ArrayBufferView");
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
