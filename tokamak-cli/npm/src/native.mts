import { decodeMessage, encodeMessage, messages, nativeError, type MessageHeader, type NativeFailure, type NativeMessage } from "@tokamakdev/plugin/native-protocol";
import type { NativePlatform } from "@tokamakdev/plugin/capabilities";

const encoder = new TextEncoder();
const decoder = new TextDecoder();

/**
 * The development invocation `ctx`'s native connection, which reaches the byte endpoints of a
 * device on `platform` through the relay on `port`. Its subscriptions are `ctx`'s `waitUntil` work.
 */
export function connectDevice(port: number, token: string, ctx: { waitUntil(promise: Promise<unknown>): void }, platform?: NativePlatform) {
  const post = async (path: string, request: Uint8Array | ReadableStream<Uint8Array>, signal?: AbortSignal) => {
    const response = await fetch(`http://127.0.0.1:${String(port)}${path}`, {
      method: "POST",
      headers: { "x-tokamak-session": token },
      body: request,
      signal,
      duplex: "half",
    } as RequestInit);
    if (!response.ok) throw new Error(`the tokamak Vite plugin answered HTTP ${String(response.status)}`);
    return response;
  };
  const result = ({ header, payload }: NativeMessage) => {
    if (header.failed) throw nativeError(JSON.parse(decoder.decode(payload)) as NativeFailure);
    return payload;
  };
  return {
    platform,
    async call(plugin: string, method: string, payload: Uint8Array, { body, signal }: { body?: ReadableStream<Uint8Array>; signal?: AbortSignal } = {}) {
      signal?.throwIfAborted();
      const header = { type: body ? "callWithBody" : "call", plugin, method };
      let message: NativeMessage;
      try {
        const request = body ? framed(encodeMessage(header, payload), body) : encodeMessage(header, payload);
        message = decodeMessage(new Uint8Array(await (await post("/call", request, signal)).arrayBuffer()));
      } catch (error) {
        if (signal?.aborted) throw signal.reason;
        throw networkError(error);
      }
      return result(message);
    },
    listen(plugin: string, method: string, payload: Uint8Array, { next, error, overflow = "error" }: {
      next(value: Uint8Array, done: boolean): void;
      error(error: DOMException, done: boolean): void;
      overflow?: string;
    }) {
      const listening = new AbortController();
      const deliver = (message: NativeMessage) => {
        let value: Uint8Array;
        try { value = result(message); }
        catch (failure) {
          try { error(failure as DOMException, message.header.done === true); } catch (thrown) { reportError(thrown); }
          return;
        }
        try { next(value, message.header.done === true); } catch (thrown) { reportError(thrown); }
      };
      const listen = async () => {
        const request = encodeMessage({ type: "subscribe", plugin, method, overflow }, payload);
        const response = await post("/subscribe", request, listening.signal);
        for await (const message of messages(response.body)) deliver(message);
      };
      ctx.waitUntil(listen().catch((failure: unknown) => {
        if (!listening.signal.aborted) {
          try { error(networkError(failure), true); } catch (thrown) { reportError(thrown); }
        }
      }));
      return () => { listening.abort(); };
    },
  };
}

/** The call message `first`, then a `chunk` message per chunk of `body` as the relay reads them. */
function framed(first: Uint8Array, body: ReadableStream<Uint8Array>): ReadableStream<Uint8Array> {
  const reader = body.getReader();
  let started = false;
  const chunk = (header: Omit<MessageHeader, "type">, payload: Uint8Array) => encodeMessage({ type: "chunk", ...header }, payload);
  return new ReadableStream({
    async pull(controller) {
      if (!started) {
        started = true;
        controller.enqueue(first);
        return;
      }
      try {
        const { value, done } = await reader.read();
        if (done) {
          controller.enqueue(chunk({ done: true }, new Uint8Array()));
          controller.close();
          return;
        }
        if (!(value instanceof Uint8Array)) throw new TypeError("A native call's body must contain Uint8Array chunks");
        controller.enqueue(chunk({}, value));
      } catch (error) {
        const failure = { name: (error as Error | undefined)?.name ?? "Error", message: String((error as Error | undefined)?.message ?? error) };
        controller.enqueue(chunk({ failed: true, done: true }, encoder.encode(JSON.stringify(failure))));
        controller.close();
      }
    },
    cancel(reason) { return reader.cancel(reason); },
  });
}

function networkError(error: unknown): DOMException {
  return new DOMException(`The device could not be reached: ${String(error)}`, "NetworkError");
}
