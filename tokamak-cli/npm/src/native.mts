import { decodeMessage, encodeMessage, messages, type NativeMessage } from "@tokamakdev/plugin/native-protocol";

/** Connects the development worker's native byte endpoints to the device relay. */
export function defineNativeCalls(port: number, token: string, waitUntil: (promise: Promise<unknown>) => void): void {
  const decoder = new TextDecoder();
  const decoded = (payload: Uint8Array): unknown => JSON.parse(decoder.decode(payload));
  const post = async (path: string, plugin: string, method: string, payload: Uint8Array, signal?: AbortSignal) => {
    const response = await fetch(`http://127.0.0.1:${String(port)}${path}`, {
      method: "POST",
      headers: { "x-tokamak-session": token },
      body: encodeMessage({ type: path.slice(1), plugin, method }, payload),
      signal,
    });
    if (!response.ok) throw new Error(`the tokamak Vite plugin answered HTTP ${String(response.status)}`);
    return response;
  };
  const result = ({ header, payload }: NativeMessage) => {
    if (header.failed) {
      const error = decoded(payload) as { name: string; message: string };
      throw new DOMException(error.message, error.name);
    }
    return payload;
  };
  const connection = {
    async call(plugin: string, method: string, payload: Uint8Array) {
      let message: NativeMessage;
      try {
        message = decodeMessage(new Uint8Array(await (await post("/call", plugin, method, payload)).arrayBuffer()));
      } catch (error) { throw networkError(error); }
      return result(message);
    },
    listen(plugin: string, method: string, payload: Uint8Array, next: (value: Uint8Array, done: boolean) => void, error: (error: DOMException, done: boolean) => void) {
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
        const response = await post("/subscribe", plugin, method, payload, listening.signal);
        for await (const message of messages(response.body)) deliver(message);
      };
      waitUntil(listen().catch((failure: unknown) => {
        if (!listening.signal.aborted) {
          try { error(networkError(failure), true); } catch (thrown) { reportError(thrown); }
        }
      }));
      return () => listening.abort();
    },
  };
  Reflect.set(globalThis, Symbol.for("tokamak.native"), connection);
}

function networkError(error: unknown): DOMException {
  return new DOMException(`The device could not be reached: ${String(error)}`, "NetworkError");
}
