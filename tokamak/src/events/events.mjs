export class EventEmitter {
  constructor() { this.__events = new Map(); }
  on(name, listener) { const list = this.__events.get(name) ?? []; list.push(listener); this.__events.set(name, list); return this; }
  addEventListener(name, listener) { return this.on(name, listener); }
  addListener(name, listener) { return this.on(name, listener); }
  once(name, listener) { const wrapped = (...args) => { this.off(name, wrapped); listener(...args); }; return this.on(name, wrapped); }
  off(name, listener) { const list = this.__events.get(name) ?? []; this.__events.set(name, list.filter((item) => item !== listener)); return this; }
  removeListener(name, listener) { return this.off(name, listener); }
  removeEventListener(name, listener) { return this.off(name, listener); }
  removeAllListeners(name) { if (name === undefined) this.__events.clear(); else this.__events.delete(name); return this; }
  emit(name, ...args) { for (const listener of [...(this.__events.get(name) ?? [])]) listener(...args); return this.__events.has(name); }
  listeners(name) { return [...(this.__events.get(name) ?? [])]; }
  listenerCount(name) { return (this.__events.get(name) ?? []).length; }
  eventNames() { return [...this.__events.keys()]; }
  setMaxListeners() { return this; }
  getMaxListeners() { return 10; }
  prependListener(name, listener) { return this.on(name, listener); }
  prependOnceListener(name, listener) { return this.once(name, listener); }
}

EventEmitter.EventEmitter = EventEmitter;
export default EventEmitter;
