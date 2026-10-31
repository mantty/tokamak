const channels = new Map();
const channelState = new WeakMap();
const tracingState = new WeakMap();

export class Channel {
  constructor(name) {
    channelState.set(this, { name: String(name), subscribers: new Set() });
  }

  get name() { return channelState.get(this).name; }
  get hasSubscribers() { return channelState.get(this).subscribers.size > 0; }
  subscribe(callback) { channelState.get(this).subscribers.add(callback); }
  unsubscribe(callback) { channelState.get(this).subscribers.delete(callback); }
  publish(message) { for (const callback of [...channelState.get(this).subscribers]) callback(message, this.name); }
}

export function channel(name) {
  const key = String(name);
  let value = channels.get(key);
  if (!value) { value = new Channel(key); channels.set(key, value); }
  return value;
}

export function subscribe(name, callback) { channel(name).subscribe(callback); }
export function unsubscribe(name, callback) { channel(name).unsubscribe(callback); }
export function hasSubscribers(name) { return channel(name).hasSubscribers; }

export class TracingChannel {
  constructor(name) {
    const prefix = String(name);
    tracingState.set(this, {
      start: channel(`${prefix}.start`),
      end: channel(`${prefix}.end`),
      asyncStart: channel(`${prefix}.asyncStart`),
      asyncEnd: channel(`${prefix}.asyncEnd`),
      error: channel(`${prefix}.error`),
    });
  }

  get start() { return tracingState.get(this).start; }
  get end() { return tracingState.get(this).end; }
  get asyncStart() { return tracingState.get(this).asyncStart; }
  get asyncEnd() { return tracingState.get(this).asyncEnd; }
  get error() { return tracingState.get(this).error; }
  subscribe(callback) { for (const value of Object.values(tracingState.get(this))) value.subscribe(callback); }
  unsubscribe(callback) { for (const value of Object.values(tracingState.get(this))) value.unsubscribe(callback); }
  get hasSubscribers() { return Object.values(tracingState.get(this)).some(value => value.hasSubscribers); }
  traceSync(fn, context, ...args) {
    this.start.publish(context);
    try { const value = fn.apply(context, args); this.end.publish(value); return value; }
    catch (error) { this.error.publish([error, context]); throw error; }
  }
  tracePromise(fn, context, ...args) {
    this.asyncStart.publish(context);
    return Promise.resolve().then(() => fn.apply(context, args)).then(value => { this.asyncEnd.publish(value); return value; }, error => { this.error.publish([error, context]); throw error; });
  }
  traceCallback(fn, context, ...args) {
    const callback = args.pop();
    this.asyncStart.publish(context);
    return fn.apply(context, [...args, (...callbackArgs) => { this.asyncEnd.publish(callbackArgs); callback?.(...callbackArgs); }]);
  }
}

export function tracingChannel(name) { return new TracingChannel(name); }
export default { Channel, channel, subscribe, unsubscribe, hasSubscribers, tracingChannel };
