import { markHostObject } from "./objects.mjs";
import { EventTarget } from "../events/events.mjs";

const internal = Symbol("performance construction");
const entryState = new WeakMap();
const now = Date.now;
const json = value => Object.assign(Object.create(null), value);

function text(value) {
  if (typeof value === "symbol") throw new TypeError("Cannot convert a Symbol to a string");
  return String(value);
}

function detailOption(options) {
  const detail = options?.detail;
  if (detail !== undefined && (detail === null || (typeof detail !== "object" && typeof detail !== "function"))) {
    throw new TypeError("Performance detail must be an object");
  }
  return detail;
}

export class PerformanceEntry {
  constructor(brand = undefined, state) {
    if (brand !== internal) throw new TypeError("Illegal constructor");
    markHostObject(this);
    entryState.set(this, state);
  }
  get name() { return entryState.get(this).name; }
  get entryType() { return entryState.get(this).entryType; }
  get startTime() { return entryState.get(this).startTime; }
  get duration() { return entryState.get(this).duration; }
  toJSON() {
    const { name, entryType, startTime, duration } = entryState.get(this);
    return json({ name, entryType, startTime, duration });
  }
}

export class PerformanceMark extends PerformanceEntry {
  constructor(name, options = {}) {
    if (arguments.length === 0) throw new TypeError("A mark name is required");
    const markName = text(name);
    const startTime = options?.startTime;
    super(internal, { name: markName, entryType: "mark", startTime: startTime === undefined ? now() : Number(startTime), duration: 0, detail: detailOption(options) });
  }
  get detail() { return entryState.get(this).detail ?? null; }
  toJSON() {
    const value = super.toJSON(), detail = entryState.get(this).detail;
    if (detail !== undefined) value.detail = detail;
    return value;
  }
}

export class PerformanceMeasure extends PerformanceEntry {
  constructor(brand = undefined, state) { super(brand, state); }
  get detail() { return entryState.get(this).detail ?? null; }
  toJSON() {
    const value = super.toJSON(), detail = entryState.get(this).detail;
    if (detail !== undefined) value.detail = detail;
    return value;
  }
}

export class PerformanceResourceTiming extends PerformanceEntry {}
for (const name of ["connectEnd", "connectStart", "decodedBodySize", "domainLookupEnd", "domainLookupStart", "encodedBodySize", "fetchStart", "initiatorType", "nextHopProtocol", "redirectEnd", "redirectStart", "requestStart", "responseEnd", "responseStart", "responseStatus", "secureConnectionStart", "transferSize", "workerStart"]) {
  Object.defineProperty(PerformanceResourceTiming.prototype, name, { enumerable: true, configurable: true,
    get() { return name === "secureConnectionStart" ? undefined : name === "initiatorType" || name === "nextHopProtocol" ? "" : 0; } });
}

const nodeTimingNames = ["nodeStart", "v8Start", "bootstrapComplete", "environment", "loopStart", "loopExit", "idleTime"];
class PerformanceNodeTiming extends PerformanceEntry {
  constructor() {
    super(internal, { name: "node", entryType: "node", startTime: 0, duration: 0 });
    for (const name of nodeTimingNames) Object.defineProperty(this, name, { enumerable: true, configurable: true, get: () => 0 });
    Object.defineProperty(this, "uvMetricsInfo", { enumerable: true, configurable: true, get: () => json({ loopCount: 0, events: 0, eventsWaiting: 0 }) });
  }
  toJSON() { return Object.assign(super.toJSON(), Object.fromEntries(nodeTimingNames.map(name => [name, 0])), { uvMetricsInfo: json({ loopCount: 0, events: 0, eventsWaiting: 0 }) }); }
}

class EventCounts {
  constructor() { markHostObject(this); }
  get size() { return 0; }
  get(_type) { return undefined; }
  has(_type) { return false; }
  entries() { return [][Symbol.iterator](); }
  keys() { return [][Symbol.iterator](); }
  values() { return [][Symbol.iterator](); }
  forEach(_callback, _receiver) {}
  [Symbol.iterator]() { return this.entries(); }
}

export class PerformanceObserverEntryList {
  constructor() { throw new TypeError("Illegal constructor"); }
  getEntries() { return []; }
  getEntriesByName(_name, _type) { return []; }
  getEntriesByType(_type) { return []; }
}

// Workerd accepts any provided callback value and does not deliver observations.
export class PerformanceObserver {
  #callback;
  static get supportedEntryTypes() { return ["measure", "mark"]; }
  constructor(callback) {
    if (arguments.length === 0) throw new TypeError("A callback argument is required");
    markHostObject(this);
    this.#callback = callback;
  }
  observe(_options) {}
  disconnect() {}
  takeRecords() { return []; }
}

export class Performance extends EventTarget {
  #entries = [];
  constructor(brand = undefined) {
    if (brand !== internal) throw new TypeError("Illegal constructor");
    super();
  }
  // Workerd uses epoch milliseconds, exactly as Date.now(), with timeOrigin 0.
  get timeOrigin() { return 0; }
  now() { return now(); }
  mark(name, options) {
    if (arguments.length === 0) throw new TypeError("A mark name is required");
    const mark = new PerformanceMark(name, options);
    this.#entries.push(mark);
    return mark;
  }
  measure(name, startOrOptions, endMark) {
    if (arguments.length === 0) throw new TypeError("A measure name is required");
    let startTime = 0, endTime = now(), detail;
    if (startOrOptions != null && (typeof startOrOptions === "object" || typeof startOrOptions === "function")) {
      const { start, end, duration } = startOrOptions;
      detail = detailOption(startOrOptions);
      if (start !== undefined) startTime = Number(start);
      if (end !== undefined) endTime = Number(end);
      else if (duration !== undefined) endTime = startTime + Number(duration);
    } else if (startOrOptions !== undefined) {
      const markTime = name => entryState.get(this.#entries.find(entry => {
        const state = entryState.get(entry);
        return state.name === name && state.entryType === "mark";
      }))?.startTime;
      startTime = markTime(text(startOrOptions)) ?? 0;
      if (endMark !== undefined) endTime = markTime(text(endMark)) ?? endTime;
    }
    const measure = new PerformanceMeasure(internal, { name: text(name), entryType: "measure", startTime, duration: endTime >= startTime ? endTime - startTime : 0, detail });
    this.#entries.push(measure);
    return measure;
  }
  clearMarks(name) { if (name !== undefined) name = text(name); this.#entries = this.#entries.filter(entry => name === undefined ? entryState.get(entry).entryType !== "mark" : entryState.get(entry).name !== name); }
  clearMeasures(name) { if (name !== undefined) name = text(name); this.#entries = this.#entries.filter(entry => name === undefined ? entryState.get(entry).entryType !== "measure" : entryState.get(entry).name !== name); }
  clearResourceTimings() { this.#entries = this.#entries.filter(entry => !["resource", "navigation"].includes(entryState.get(entry).entryType)); }
  getEntries() { return [...this.#entries]; }
  getEntriesByName(name, type) { name = text(name); if (type !== undefined) type = text(type); return this.#entries.filter(entry => { const state = entryState.get(entry); return state.name === name && (type === undefined || state.entryType === type); }); }
  getEntriesByType(type) { type = text(type); return this.#entries.filter(entry => entryState.get(entry).entryType === type); }
  get eventCounts() { return new EventCounts(); }
  eventLoopUtilization() { return { idle: 0, active: 0, utilization: 0 }; }
  get nodeTiming() { return new PerformanceNodeTiming(); }
  markResourceTiming() {}
  setResourceTimingBufferSize(_size) {}
  timerify(callback) { if (typeof callback !== "function") throw new TypeError("The callback argument must be of type function"); return callback; }
  toJSON() { return json({ timeOrigin: 0 }); }
}

for (const Type of [PerformanceEntry, PerformanceMark, PerformanceMeasure, PerformanceResourceTiming, PerformanceNodeTiming, EventCounts, PerformanceObserverEntryList, PerformanceObserver, Performance]) {
  Object.defineProperty(Type.prototype, Symbol.toStringTag, { value: Type.name, configurable: true });
}

export const performance = new Performance(internal);
