import { decodeMessage, encodeMessage, type RoutingHeader } from "@tokamakdev/plugin/native-protocol";
import type { RawData } from "ws";

const encoder = new TextEncoder();
const decoder = new TextDecoder();

/** A native message with its payload as a field, as the JSON endpoint fixtures write it. */
export interface DeviceMessage extends RoutingHeader {
  arguments?: unknown;
  event?: unknown;
  reply?: unknown;
  result?: { value?: unknown; error?: unknown; done?: boolean };
  message?: string;
}

// JSON endpoint fixtures share the binary transport used by native devices.
export function encodeDevice(message: DeviceMessage): Uint8Array<ArrayBuffer> {
  const { arguments: args, event, reply, result, message: error, ...header } = message;
  if (result) {
    header.failed = result.error !== undefined;
    header.done = result.done;
  }
  const value = result ? result.error ?? result.value : event ?? reply ?? args ?? null;
  return encodeMessage(header, encoder.encode(header.type === "error" ? error : JSON.stringify(value)));
}

/** The bytes of a WebSocket message, which ws gives as one buffer or, for a fragmented message, several. */
export function rawBytes(data: RawData): Uint8Array {
  return Array.isArray(data) ? Buffer.concat(data) : new Uint8Array(data);
}

/** The message a WebSocket message carries. */
export function decodeDevice(data: RawData): DeviceMessage {
  const { header, payload } = decodeMessage(rawBytes(data));
  const message: DeviceMessage = header;
  const text = decoder.decode(payload);
  if (message.type === "call" || message.type === "subscribe") message.arguments = text ? JSON.parse(text) : null;
  if (message.type === "reply") message.reply = text ? JSON.parse(text) : null;
  if (message.type === "error") message.message = text;
  return message;
}
