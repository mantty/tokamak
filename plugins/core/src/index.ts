interface NativeTransport {
  postMessage(message: string): void;
  onmessage: ((event: { data: string }) => void) | null;
}

interface NativeError {
  name: string;
  message: string;
}

interface NativeResponse {
  session: string;
  id: number;
  value?: unknown;
  error?: NativeError;
  done: boolean;
}

interface PendingCall {
  kind: "call";
  resolve(value: unknown): void;
  reject(error: DOMException): void;
}

interface PendingSubscription {
  kind: "subscription";
  next(value: unknown): void;
  error(error: DOMException): void;
}

type Pending = PendingCall | PendingSubscription;

/** Calls `method` of `plugin` from the Worker, resolving with its value. */
type WorkerCall = (plugin: string, method: string, arguments_: unknown) => Promise<unknown>;

/** Subscribes to `method` of `plugin` from the Worker, returning the function that unsubscribes. */
type WorkerListen = (
  plugin: string,
  method: string,
  arguments_: unknown,
  next: (value: unknown) => void,
  error: (error: DOMException) => void,
) => () => void;

declare global {
  /** The page's native transport, which the shells define. */
  var __tokamakNative: NativeTransport | undefined;
  /** The Worker's native calls, which the runtime and `tok dev` define. */
  var __tokamakNativeCall: WorkerCall | undefined;
  var __tokamakNativeListen: WorkerListen | undefined;
}

let nextRequestId = 1;
const session = `${String(Date.now())}-${String(Math.random())}`;
let connected: NativeTransport | undefined;
const pending = new Map<number, Pending>();

/** The base class of a plugin's API, which the page and the Worker call alike. */
export abstract class Plugin {
  protected constructor(private readonly pluginId: string) {}

  protected get hasNativeTransport(): boolean {
    return pageTransport() !== undefined || globalThis.__tokamakNativeCall !== undefined;
  }

  protected call<T>(method: string, arguments_: unknown = null): Promise<T> {
    const page = pageTransport();
    if (page) {
      return new Promise<T>((resolve, reject) => {
        send(page, this.pluginId, method, arguments_, { kind: "call", resolve, reject });
      });
    }
    const call = globalThis.__tokamakNativeCall;
    if (call) return call(this.pluginId, method, arguments_) as Promise<T>;
    return Promise.reject(unavailable());
  }

  // eslint-disable-next-line @typescript-eslint/no-unnecessary-type-parameters -- T types the native values, as in call.
  protected listen<T>(
    method: string,
    next: (value: T) => void,
    error: (error: DOMException) => void,
    arguments_: unknown = null,
  ): () => void {
    const page = pageTransport();
    if (page) {
      const id = send(page, this.pluginId, method, arguments_, {
        kind: "subscription",
        next,
        error,
      });
      return () => {
        if (!pending.delete(id)) return;
        page.postMessage(JSON.stringify({ type: "cancel", session, id }));
      };
    }
    const listen = globalThis.__tokamakNativeListen;
    if (listen) return listen(this.pluginId, method, arguments_, next as (value: unknown) => void, error);
    throw unavailable();
  }
}

function unavailable(): DOMException {
  return new DOMException("Native plugin transport is unavailable", "NotSupportedError");
}

/** The page's native transport, connected on first use. */
function pageTransport(): NativeTransport | undefined {
  const transport = globalThis.__tokamakNative;
  if (!transport || typeof transport.postMessage !== "function") return undefined;
  if (connected !== transport) connect(transport);
  return transport;
}

function connect(transport: NativeTransport): void {
  if (!connected) {
    const page: { addEventListener?: (type: string, listener: () => void) => void } = globalThis;
    page.addEventListener?.("pagehide", cancelSubscriptions);
  }
  connected = transport;
  transport.onmessage = ({ data }) => {
    receive(JSON.parse(data) as NativeResponse);
  };
  transport.postMessage(JSON.stringify({ type: "reset", session }));
}

/** Posts a call or subscription, keeping `request` pending unless posting throws. */
function send(
  transport: NativeTransport,
  plugin: string,
  method: string,
  arguments_: unknown,
  request: Pending,
): number {
  const id = nextRequestId++;
  const type = request.kind === "call" ? "call" : "subscribe";
  pending.set(id, request);
  try {
    transport.postMessage(
      JSON.stringify({ type, session, id, plugin, method, arguments: arguments_ }),
    );
  } catch (error) {
    pending.delete(id);
    throw error;
  }
  return id;
}

function receive(response: NativeResponse): void {
  if (response.session !== session) return;
  const request = pending.get(response.id);
  if (!request) return;
  if (response.done) pending.delete(response.id);

  if (response.error) {
    const error = new DOMException(response.error.message, response.error.name);
    if (request.kind === "call") request.reject(error);
    else request.error(error);
    return;
  }

  if (request.kind === "call") request.resolve(response.value);
  else request.next(response.value);
}

function cancelSubscriptions(): void {
  if (!connected) return;
  for (const [id, request] of pending) {
    if (request.kind !== "subscription") continue;
    pending.delete(id);
    connected.postMessage(JSON.stringify({ type: "cancel", session, id }));
  }
}
