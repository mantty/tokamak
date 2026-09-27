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
      const transport = requireNativeTransport();
      const id = nextRequestId++;
      pending.set(id, {
        kind: "call",
        resolve: (value) => {
          resolve(value as T);
        },
        reject,
      });
      post(
        transport,
        {
          type: "call",
          session,
          id,
          plugin: this.pluginId,
          method,
          arguments: arguments_,
        },
        id,
      );
    });
  }

  protected subscribe(
    method: string,
    next: (value: unknown) => void,
    error: (error: DOMException) => void,
    arguments_: unknown = null,
  ): () => void {
    const transport = requireNativeTransport();
    const id = nextRequestId++;
    pending.set(id, {
      kind: "subscription",
      next: (value) => {
        next(value);
      },
      error,
    });
    post(
      transport,
      {
        type: "subscribe",
        session,
        id,
        plugin: this.pluginId,
        method,
        arguments: arguments_,
      },
      id,
    );
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

function post(transport: NativeTransport, message: object, id: number): void {
  try {
    transport.postMessage(JSON.stringify(message));
  } catch (error) {
    pending.delete(id);
    throw error;
  }
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
