/** Browser and service worker globals the notifications plugin uses, in memory. */

export interface ShownNotification {
  readonly title: string;
  readonly body: string;
  readonly tag: string;
  readonly data: unknown;
  closed: boolean;
  close(): void;
}

export class FakePushSubscription {
  readonly options = { userVisibleOnly: true, applicationServerKey: null };
  unsubscribed = false;

  constructor(readonly endpoint: string) {}

  toJSON() {
    return { endpoint: this.endpoint, keys: { p256dh: `${this.endpoint}-p256dh`, auth: "auth" } };
  }

  unsubscribe(): Promise<boolean> {
    this.unsubscribed = true;
    return Promise.resolve(true);
  }
}

export class FakeRegistration {
  readonly shown: ShownNotification[] = [];
  subscription: FakePushSubscription | null = null;
  readonly subscribeOptions: unknown[] = [];

  readonly pushManager = {
    getSubscription: () => Promise.resolve(this.subscription),
    subscribe: (options: unknown) => {
      this.subscribeOptions.push(options);
      this.subscription = new FakePushSubscription("https://push.example/new");
      return Promise.resolve(this.subscription);
    },
  };

  showNotification(title: string, options: { body?: string; tag: string; data: unknown }) {
    const notification: ShownNotification = {
      title,
      body: options.body ?? "",
      tag: options.tag,
      data: options.data,
      closed: false,
      close() {
        this.closed = true;
      },
    };
    this.shown.push(notification);
    return Promise.resolve();
  }

  getNotifications(filter: { tag?: string } = {}) {
    return Promise.resolve(
      this.shown.filter(
        (notification) =>
          !notification.closed && (filter.tag === undefined || notification.tag === filter.tag),
      ),
    );
  }
}

export class FakeCacheStorage {
  readonly entries = new Map<string, Map<string, string>>();

  open(name: string) {
    const entries = this.entries.get(name) ?? new Map<string, string>();
    this.entries.set(name, entries);
    return Promise.resolve({
      put: async (url: string, response: Response) => {
        entries.set(url, await response.text());
      },
      keys: () => Promise.resolve([...entries.keys()]),
      match: (url: string) => {
        const body = entries.get(url);
        return Promise.resolve(body === undefined ? undefined : new Response(body));
      },
      delete: (url: string) => Promise.resolve(entries.delete(url)),
    });
  }
}

/** Replaces `name` on the global object until `restore` runs. */
export function replaceGlobal(name: string, value: unknown): () => void {
  const original = Object.getOwnPropertyDescriptor(globalThis, name);
  Object.defineProperty(globalThis, name, { configurable: true, writable: true, value });
  return () => {
    if (original) Object.defineProperty(globalThis, name, original);
    else Reflect.deleteProperty(globalThis, name);
  };
}
