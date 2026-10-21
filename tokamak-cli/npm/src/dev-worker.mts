import { createRequire } from "node:module";
import type { IncomingMessage } from "node:http";
import { Readable } from "node:stream";
import { pathToFileURL } from "node:url";
import type { WebSocket } from "ws";

export interface WorkerSocket {
  accept(): void;
  close(code: number, reason: string): void;
}

interface WorkerResponse extends Response { webSocket?: WorkerSocket | null }

interface Miniflare {
  readonly ready: Promise<unknown>;
  dispatchFetch(url: string, init: Record<string, unknown>): Promise<WorkerResponse>;
  dispose(): Promise<void>;
}

interface MiniflareModule {
  Miniflare: new (options: unknown) => Miniflare;
  convertV4MiniflareOptions(options: unknown): unknown;
  getDefaultDevRegistryPath(): string;
  coupleWebSocket(socket: WebSocket, pair: WorkerSocket): Promise<void>;
}

const CALLER = `export default {
  async fetch(request, env) {
    if (new URL(request.url).pathname.startsWith("/_tokamak/")) return env.TOKAMAK.fetch(request);
    const name = request.headers.get("x-tokamak-event");
    try {
      const { reply } = await env.EVENTS.dispatch(name, new Uint8Array(await request.arrayBuffer()));
      return new Response(reply);
    } catch (error) {
      return new Response(error?.stack ?? String(error), { status: 500 });
    }
  },
};`;

/** The native Vite worker, reached through Miniflare's development registry. */
export class DevWorker {
  #session: Promise<{ runtime: Miniflare; module: MiniflareModule }> | undefined;
  #disposed = false;

  constructor(private readonly name: string | undefined) {}

  /** Delivers the event `name` with its encoded data, resolving to the encoded reply. */
  async dispatch(name: string, event: Uint8Array): Promise<Uint8Array> {
    const { runtime } = await this.#connect();
    const response = await runtime.dispatchFetch("http://tokamak/", {
      method: "POST", headers: { "x-tokamak-event": name }, body: event,
    });
    if (!response.ok) throw new Error(await response.text());
    return new Uint8Array(await response.arrayBuffer());
  }

  async fetch(request: IncomingMessage, signal: AbortSignal): Promise<WorkerResponse> {
    const { runtime } = await this.#connect();
    const headers = new Headers();
    for (const [name, value] of Object.entries(request.headers)) {
      if (value !== undefined) headers.set(name, [value].flat().join(", "));
    }
    const host = headers.get("x-forwarded-host") ?? headers.get("host");
    const protocol = headers.get("x-forwarded-proto") ?? "http";
    const url = new URL(request.url ?? "/", `${protocol}://${host}`);
    for (const name of ["x-tokamak-session", "x-tokamak-dev-socket", "x-forwarded-host", "x-forwarded-proto"]) headers.delete(name);
    return runtime.dispatchFetch(url.href, {
      method: request.method, headers, signal,
      body: request.method === "GET" || request.method === "HEAD" ? undefined : Readable.toWeb(request),
      duplex: "half",
    });
  }

  async couple(socket: WebSocket, pair: WorkerSocket): Promise<void> {
    const { module } = await this.#connect();
    await module.coupleWebSocket(socket, pair);
  }

  /** Disposes the runtime; later deliveries reject rather than start another. */
  async dispose(): Promise<void> {
    this.#disposed = true;
    await (await this.#session?.catch(() => undefined))?.runtime.dispose();
  }

  #connect(): Promise<{ runtime: Miniflare; module: MiniflareModule }> {
    if (this.#disposed) return Promise.reject(new Error("the development server has closed"));
    return this.#session ??= this.#start().catch((error: unknown) => {
      this.#session = undefined;
      throw error;
    });
  }

  async #start(): Promise<{ runtime: Miniflare; module: MiniflareModule }> {
    if (!this.name) throw new Error("the Wrangler configuration names no native Worker");
    const require = createRequire(import.meta.resolve("@cloudflare/vite-plugin"));
    const module = (await import(pathToFileURL(require.resolve("miniflare")).href)) as MiniflareModule;
    const runtime = new module.Miniflare(module.convertV4MiniflareOptions({
      modules: true,
      script: CALLER,
      compatibilityDate: "2025-01-01",
      serviceBindings: {
        TOKAMAK: { name: this.name },
        EVENTS: { name: this.name, entrypoint: "TokamakEvents" },
      },
      unsafeDevRegistryPath: module.getDefaultDevRegistryPath(),
    }));
    try { await runtime.ready; }
    catch (error) { await runtime.dispose(); throw error; }
    return { runtime, module };
  }
}
