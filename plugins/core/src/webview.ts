import { decodeJson, encodeJson, type JsonValue } from "./codec.js";
import type { NativeConnection } from "./context.js";

/** Controls the app's current WebView. Operations reject when no page is available. */
export interface WebView {
  /** Starts navigation to an app URL or path. Resolves when navigation is accepted. */
  navigate(url: string): Promise<void>;
  /** The current page's absolute URL. */
  getCurrentPage(): Promise<string>;
  /** Dispatches a window `message` event in the current page. Messages are not queued. */
  postMessage(data: JsonValue): Promise<void>;
}

export function webview(connection: NativeConnection): WebView {
  const call = async <T>(method: string, value: unknown = null): Promise<T> =>
    decodeJson<T>(await connection.call("tokamak", `webview.${method}`, encodeJson(value)));
  return Object.freeze({
    navigate: async (url: string) => { await call<null>("navigate", url); },
    getCurrentPage: () => call<string>("getCurrentPage"),
    postMessage: async (data: JsonValue) => { await call<null>("postMessage", data); },
  });
}
