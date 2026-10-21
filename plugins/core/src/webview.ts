import { decodeJson, encodeJson, type JsonValue } from "./codec.js";
import type { NativeConnection } from "./context.js";

/** Controls the app's WebView. Operations reject with `InvalidStateError` when what they need is unavailable. */
export interface WebView {
  /**
   * Starts navigation to an app URL, or a path relative to the current page or the app origin.
   * Resolves when navigation is accepted. Needs the WebView, not a loaded page.
   */
  navigate(url: string): Promise<void>;
  /** The committed page's absolute URL, or null when no page is loaded. */
  getCurrentPage(): Promise<string | null>;
  /** Dispatches a window `message` event in the current page. Messages are not queued. */
  postMessage(data: JsonValue): Promise<void>;
}

export function webview(connection: NativeConnection): WebView {
  const call = async <T>(method: string, value: unknown = null): Promise<T> =>
    decodeJson<T>(await connection.call("tokamak", `webview.${method}`, encodeJson(value)));
  return Object.freeze({
    navigate: async (url: string) => { await call<null>("navigate", url); },
    getCurrentPage: () => call<string | null>("getCurrentPage"),
    postMessage: async (data: JsonValue) => { await call<null>("postMessage", data); },
  });
}
