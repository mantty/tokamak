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

declare global {
  var __tokamakNative: NativeTransport | undefined;
  var __tokamakReceive: ((response: NativeResponse) => void) | undefined;
}

let nextRequestId = 1;
const session = `${String(Date.now())}-${String(Math.random())}`;
let connected: NativeTransport | undefined;
const pending = new Map<number, Pending>();

export abstract class FrontendPlugin {
  protected constructor(private readonly pluginId: string) {}

  protected get hasNativeTransport(): boolean {
    return nativeTransport() !== undefined;
  }

  protected call<T>(method: string, arguments_: unknown = null): Promise<T> {
    return new Promise<T>((resolve, reject) => {
      send("call", this.pluginId, method, arguments_, {
        kind: "call",
        resolve: (value) => {
          resolve(value as T);
        },
        reject,
      });
    });
  }

  protected listen(
    method: string,
    next: (value: unknown) => void,
    error: (error: DOMException) => void,
    arguments_: unknown = null,
  ): () => void {
    const { transport, id } = send("subscribe", this.pluginId, method, arguments_, {
      kind: "subscription",
      next: (value) => {
        next(value);
      },
      error,
    });
    return () => {
      if (!pending.delete(id)) return;
      transport.postMessage(JSON.stringify({ type: "cancel", session, id }));
    };
  }
}

function nativeTransport(): NativeTransport | undefined {
  const transport = globalThis.__tokamakNative;
  if (!transport || typeof transport.postMessage !== "function") return undefined;
  if (connected !== transport) {
    connected = transport;
    transport.onmessage = ({ data }) => {
      receive(JSON.parse(data) as NativeResponse);
    };
    globalThis.__tokamakReceive = receive;
    transport.postMessage(JSON.stringify({ type: "reset", session }));
  }
  return transport;
}

function requireNativeTransport(): NativeTransport {
  const transport = nativeTransport();
  if (transport) return transport;
  throw new DOMException("Native plugin transport is unavailable", "NotSupportedError");
}

/** Posts a call or subscription, keeping `request` pending unless posting throws. */
function send(
  type: "call" | "subscribe",
  plugin: string,
  method: string,
  arguments_: unknown,
  request: Pending,
): { transport: NativeTransport; id: number } {
  const transport = requireNativeTransport();
  const id = nextRequestId++;
  pending.set(id, request);
  try {
    transport.postMessage(
      JSON.stringify({ type, session, id, plugin, method, arguments: arguments_ }),
    );
  } catch (error) {
    pending.delete(id);
    throw error;
  }
  return { transport, id };
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

const eventTarget: {
  addEventListener?: (type: string, listener: () => void) => void;
} = globalThis;

eventTarget.addEventListener?.("pagehide", () => {
  if (!connected) return;
  for (const [id, request] of pending) {
    if (request.kind !== "subscription") continue;
    pending.delete(id);
    connected.postMessage(JSON.stringify({ type: "cancel", session, id }));
  }
});
