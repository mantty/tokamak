// Defines the development Worker's native calls, which reach the device through the tokamak Vite
// plugin. The plugin's entry Worker transform imports this module in development only.

import type { PluginResult } from "./dev-socket.mjs";

/**
 * Defines `__tokamakNativeCall` and `__tokamakNativeListen`, which post to the plugin's loopback
 * `port` with the session `token`. `waitUntil` keeps each listener's invocation running, as
 * `waitUntil` from `cloudflare:workers` does.
 */
export function defineNativeCalls(port: number, token: string, waitUntil: (promise: Promise<unknown>) => void): void {
  const post = async (path: string, plugin: string, method: string, arguments_: unknown, signal?: AbortSignal) => {
    const response = await fetch(`http://127.0.0.1:${String(port)}${path}`, {
      method: "POST",
      headers: { "x-tokamak-session": token },
      body: JSON.stringify({ plugin, method, arguments: arguments_ }),
      signal,
    });
    if (!response.ok) throw new Error(`the tokamak Vite plugin answered HTTP ${String(response.status)}`);
    return response;
  };

  globalThis.__tokamakNativeCall = async (plugin, method, arguments_ = null) => {
    let result: PluginResult;
    try {
      result = (await (await post("/call", plugin, method, arguments_)).json()) as PluginResult;
    } catch (error) {
      throw networkError(error);
    }
    if (result.error) throw new DOMException(result.error.message, result.error.name);
    return result.value;
  };

  globalThis.__tokamakNativeListen = (plugin, method, arguments_, next, error) => {
    const listening = new AbortController();
    const deliver = (result: PluginResult) => {
      try {
        if (result.error) error(new DOMException(result.error.message, result.error.name));
        else next(result.value);
      } catch (thrown) {
        reportError(thrown);
      }
    };
    const listen = async () => {
      const response = await post("/subscribe", plugin, method, arguments_ ?? null, listening.signal);
      for await (const result of lines(response)) deliver(result);
    };
    waitUntil(
      listen().catch((failure: unknown) => {
        if (!listening.signal.aborted) deliver({ error: networkError(failure), done: true });
      }),
    );
    return () => {
      listening.abort();
    };
  };
}

/** Each line of `response`'s body, parsed as a result. */
async function* lines(response: Response): AsyncGenerator<PluginResult> {
  const decoder = new TextDecoder();
  let text = "";
  for await (const chunk of response.body ?? []) {
    text += decoder.decode(chunk, { stream: true });
    const complete = text.split("\n");
    text = complete.pop() ?? "";
    for (const line of complete) yield JSON.parse(line) as PluginResult;
  }
}

function networkError(error: unknown): DOMException {
  return new DOMException(`The device could not be reached: ${String(error)}`, "NetworkError");
}
