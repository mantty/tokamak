/** A native transport that records the page's messages and replies as the native bridges do. */
export class FakeNativeTransport {
  /** Every message the page posted, parsed. */
  readonly sent: Record<string, unknown>[] = [];
  onmessage: ((event: { data: string }) => void) | null = null;

  postMessage(message: string): void {
    this.sent.push(JSON.parse(message) as Record<string, unknown>);
  }

  /** The last message's type, plugin, method and arguments. */
  lastRequest(): Record<string, unknown> {
    const { type, plugin, method, arguments: arguments_ } = this.sent.at(-1) ?? {};
    return { type, plugin, method, arguments: arguments_ };
  }

  /** Replies to the last message. */
  respond(reply: { value?: unknown; error?: { name: string; message: string } }, done = true): void {
    const { session, id } = this.sent.at(-1) ?? {};
    this.deliver({ session, id, done, ...reply });
  }

  /** Delivers `response` to the page. */
  deliver(response: Record<string, unknown>): void {
    this.onmessage?.({ data: JSON.stringify(response) });
  }
}

/** Installs a new fake as the page's native transport. */
export function connectNative(): FakeNativeTransport {
  const transport = new FakeNativeTransport();
  globalThis.__tokamakNative = transport;
  return transport;
}

/** Removes the page's native transport. */
export function disconnectNative(): void {
  Reflect.deleteProperty(globalThis, "__tokamakNative");
}
