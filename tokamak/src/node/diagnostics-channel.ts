import { callFunction, callMethod } from "../globals/objects.ts";

const channels = new Map<string, Channel>();

export class Channel {
  #name: string;
  #subscribers = new Set<unknown>();

  constructor(name: unknown) {
    this.#name = String(name);
  }

  get name() { return this.#name; }
  get hasSubscribers() { return this.#subscribers.size > 0; }
  subscribe(callback: unknown): void { this.#subscribers.add(callback); }
  unsubscribe(callback: unknown): void { this.#subscribers.delete(callback); }
  publish(message: unknown): void { for (const callback of [...this.#subscribers]) callFunction(callback, undefined, message, this.name); }
}

export function channel(name: unknown): Channel {
  const key = String(name);
  let value = channels.get(key);
  if (!value) { value = new Channel(key); channels.set(key, value); }
  return value;
}

export function subscribe(name: unknown, callback: unknown): void { channel(name).subscribe(callback); }
export function unsubscribe(name: unknown, callback: unknown): void { channel(name).unsubscribe(callback); }
export function hasSubscribers(name: unknown): boolean { return channel(name).hasSubscribers; }

export class TracingChannel {
  #channels: { start: Channel; end: Channel; asyncStart: Channel; asyncEnd: Channel; error: Channel };

  constructor(name: unknown) {
    const prefix = String(name);
    this.#channels = {
      start: channel(`${prefix}.start`),
      end: channel(`${prefix}.end`),
      asyncStart: channel(`${prefix}.asyncStart`),
      asyncEnd: channel(`${prefix}.asyncEnd`),
      error: channel(`${prefix}.error`),
    };
  }

  get start() { return this.#channels.start; }
  get end() { return this.#channels.end; }
  get asyncStart() { return this.#channels.asyncStart; }
  get asyncEnd() { return this.#channels.asyncEnd; }
  get error() { return this.#channels.error; }
  subscribe(callback: unknown): void { for (const value of Object.values(this.#channels)) value.subscribe(callback); }
  unsubscribe(callback: unknown): void { for (const value of Object.values(this.#channels)) value.unsubscribe(callback); }
  get hasSubscribers() { return Object.values(this.#channels).some(value => value.hasSubscribers); }
  traceSync(fn: unknown, context: unknown, ...args: unknown[]): unknown {
    this.start.publish(context);
    try { const value = callMethod(fn, "apply", context, args); this.end.publish(value); return value; }
    catch (error) { this.error.publish([error, context]); throw error; }
  }
  tracePromise(fn: unknown, context: unknown, ...args: unknown[]): Promise<unknown> {
    this.asyncStart.publish(context);
    return Promise.resolve().then(() => callMethod(fn, "apply", context, args)).then(value => { this.asyncEnd.publish(value); return value; }, error => { this.error.publish([error, context]); throw error; });
  }
  traceCallback(fn: unknown, context: unknown, ...args: unknown[]): unknown {
    const callback = args.pop();
    this.asyncStart.publish(context);
    return callMethod(fn, "apply", context, [...args, (...callbackArgs: unknown[]) => {
      this.asyncEnd.publish(callbackArgs);
      if (callback != null) callFunction(callback, undefined, ...callbackArgs);
    }]);
  }
}

export function tracingChannel(name: unknown): TracingChannel { return new TracingChannel(name); }
export default { Channel, channel, subscribe, unsubscribe, hasSubscribers, tracingChannel };
