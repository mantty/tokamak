import { TextDecoder, TextEncoder } from "../streams/text.mjs";
import { Buffer } from "./buffer.mjs";

const states = new WeakMap();

function normalizeEncoding(value) {
  const encoding = String(value).toLowerCase();
  if (["utf8", "utf-8"].includes(encoding)) return "utf8";
  if (["utf16le", "utf-16le", "ucs2", "ucs-2"].includes(encoding)) return "utf16le";
  if (["latin1", "binary"].includes(encoding)) return "latin1";
  if (["ascii"].includes(encoding)) return "ascii";
  if (["base64", "base64url", "hex"].includes(encoding)) return encoding;
  const error = new TypeError(`Unknown encoding: ${encoding}`);
  error.code = "ERR_UNKNOWN_ENCODING";
  throw error;
}

function bytes(value) {
  if (typeof value === "string") return new TextEncoder().encode(value);
  if (value instanceof ArrayBuffer) return new Uint8Array(value);
  if (ArrayBuffer.isView(value)) return new Uint8Array(value.buffer, value.byteOffset, value.byteLength);
  throw new TypeError("The \"buffer\" argument must be of type string or an instance of Buffer, TypedArray, or DataView");
}

function utf8Length(value) {
  return value < 0x80 ? 1 : value >= 0xc2 && value <= 0xdf ? 2 : value >= 0xe0 && value <= 0xef ? 3 : value >= 0xf0 && value <= 0xf4 ? 4 : 1;
}

function utf8Pending(input) {
  let continuation = 0;
  while (continuation < input.length && input[input.length - continuation - 1] >= 0x80 && input[input.length - continuation - 1] <= 0xbf) continuation += 1;
  const start = input.length - continuation - 1;
  if (start < 0) return { start: input.length, total: 0 };
  const total = utf8Length(input[start]);
  return total > continuation + 1 && total > 1 ? { start, total } : { start: input.length, total: 0 };
}

function remember(state, input) {
  state.lastChar.fill(0);
  if (input.length) state.lastChar.set(input.slice(Math.max(0, input.length - 4)));
}

function rememberUtf8(state, input, pending) {
  if (pending.total) {
    state.lastChar.fill(0);
    state.lastChar.set(input.slice(pending.start));
    return;
  }
  let start = input.length - 1;
  while (start > 0 && input[start] >= 0x80 && input[start] <= 0xbf) start -= 1;
  const total = start >= 0 ? utf8Length(input[start]) : 0;
  remember(state, total > 1 && total === input.length - start ? input.slice(start) : input.slice(-1));
}

function utf16Chars(input) {
  let text = "";
  for (let index = 0; index + 1 < input.length; index += 2) text += String.fromCharCode(input[index] | (input[index + 1] << 8));
  return text;
}

function decodeBytes(state, input) {
  if (state.encoding === "latin1") return String.fromCharCode(...input);
  if (state.encoding === "ascii") return String.fromCharCode(...input.map(value => value & 0x7f));
  if (state.encoding === "utf16le") return utf16Chars(input);
  if (state.encoding === "hex") return [...input].map(value => value.toString(16).padStart(2, "0")).join("");
  if (state.encoding === "base64" || state.encoding === "base64url") return Buffer.from(input).toString(state.encoding);
  return new TextDecoder("utf-8", { ignoreBOM: true }).decode(input);
}

export class StringDecoder {
  constructor(encoding = "utf8") {
    const state = { encoding: normalizeEncoding(encoding), pending: new Uint8Array(), lastNeed: 0, lastTotal: 0, lastChar: Buffer.alloc(4) };
    states.set(this, state);
    this.encoding = state.encoding;
  }

  get lastNeed() { return states.get(this).lastNeed; }
  get lastTotal() { return states.get(this).lastTotal; }
  get lastChar() { return states.get(this).lastChar; }

  write(value) {
    const state = states.get(this);
    const input = new Uint8Array([...state.pending, ...bytes(value)]);
    let complete = input.length;
    if (state.encoding === "base64" || state.encoding === "base64url") {
      complete = input.length - (input.length % 3);
      state.lastNeed = 0;
      state.lastTotal = 0;
      remember(state, input.slice(complete).length ? input.slice(complete) : input.slice(-Math.min(3, input.length)));
      state.pending = input.slice(complete);
      return decodeBytes(state, input.slice(0, complete));
    } else if (state.encoding === "utf8") {
      const pending = utf8Pending(input);
      complete = pending.start;
      state.lastTotal = pending.total;
      state.lastNeed = pending.total ? pending.total - (input.length - pending.start) : 0;
      rememberUtf8(state, input, pending);
    } else if (state.encoding === "utf16le") {
      complete = input.length;
      if (complete % 2) {
        complete -= 1;
        state.lastNeed = 1;
        state.lastTotal = 2;
      } else if (complete >= 2 && (input[complete - 1] & 0xfc) === 0xd8) {
        complete -= 2;
        state.lastNeed = 2;
        state.lastTotal = 4;
      } else {
        state.lastNeed = 0;
        state.lastTotal = 0;
      }
      if (state.lastNeed) remember(state, input.slice(complete));
    } else {
      state.lastNeed = 0;
      state.lastTotal = 0;
      state.lastChar.fill(0);
    }
    state.pending = input.slice(complete);
    return decodeBytes(state, input.slice(0, complete));
  }

  end(value) {
    const state = states.get(this);
    const output = value === undefined ? "" : this.write(value);
    const tail = state.pending;
    state.pending = new Uint8Array();
    state.lastNeed = 0;
    state.lastTotal = 0;
    if (!tail.length) return output;
    if (state.encoding === "utf8") return output + "\ufffd";
    return output + decodeBytes(state, tail);
  }

  text(value) { return this.write(value); }
}

export default { StringDecoder };
