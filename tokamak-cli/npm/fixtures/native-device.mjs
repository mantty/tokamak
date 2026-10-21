import { decodeMessage, encodeMessage } from "@tokamakdev/plugin/native-protocol";

const encoder = new TextEncoder();
const decoder = new TextDecoder();

// JSON endpoint fixtures share the binary transport used by native devices.
export function encodeDevice(message) {
  const { arguments: args, event, reply, result, message: error, ...header } = message;
  if (result) {
    header.failed = result.error !== undefined;
    header.done = result.done;
  }
  const value = result ? result.error ?? result.value : event ?? reply ?? args ?? null;
  return encodeMessage(header, encoder.encode(header.type === "error" ? error : JSON.stringify(value)));
}

export function decodeDevice(bytes) {
  const { header, payload } = decodeMessage(new Uint8Array(bytes));
  const text = decoder.decode(payload);
  if (header.type === "call" || header.type === "subscribe") header.arguments = text ? JSON.parse(text) : null;
  if (header.type === "reply") header.reply = text ? JSON.parse(text) : null;
  if (header.type === "error") header.message = text;
  return header;
}
