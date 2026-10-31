import { isObject, markHostObject, objectOf } from "./objects.ts";
import { EventTarget } from "../events/web.ts";
import { string } from "./conversions.ts";

interface EntryState {
  name: string;
  entryType: string;
  startTime: number;
  duration: number;
  detail?: unknown;
}

const internal = Symbol("performance construction");
const now = Date.now;
const json = (value: Record<string, unknown>): Record<string, unknown> => Object.assign(Object.create(null), value);

// The option `name` of `options`, which may be any value.
function option(options: unknown, name: string): unknown {
  return options == null ? undefined : objectOf(options)[name];
}

function detailOption(options: unknown): unknown {
  const detail = option(options, "detail");
  if (detail !== undefined && (detail === null || (typeof detail !== "object" && typeof detail !== "function"))) {
    throw new TypeError("Performance detail must be an object");
  }
  return detail;
}

// `value`, an entry's JSON, with the entry's detail when it has one.
function detailJson(entry: PerformanceEntry, value: Record<string, unknown>): Record<string, unknown> {
  const { detail } = stateOf(entry);
  if (detail !== undefined) value.detail = detail;
  return value;
}

let stateOf: (entry: PerformanceEntry) => EntryState;

export class PerformanceEntry {
  #state: EntryState;
  constructor(brand: unknown = undefined, state?: EntryState) {
    if (brand !== internal || state === undefined) throw new TypeError("Illegal constructor");
    markHostObject(this);
    this.#state = state;
  }
  get name() { return this.#state.name; }
  get entryType() { return this.#state.entryType; }
  get startTime() { return this.#state.startTime; }
  get duration() { return this.#state.duration; }
  toJSON(): Record<string, unknown> {
    const { name, entryType, startTime, duration } = this.#state;
    return json({ name, entryType, startTime, duration });
  }
  static { stateOf = entry => entry.#state; }
}

export class PerformanceMark extends PerformanceEntry {
  constructor(name: unknown, options: unknown = {}) {
    if (arguments.length === 0) throw new TypeError("A mark name is required");
    const markName = string(name);
    const startTime = option(options, "startTime");
    super(internal, { name: markName, entryType: "mark", startTime: startTime === undefined ? now() : Number(startTime), duration: 0, detail: detailOption(options) });
  }
  get detail() { return stateOf(this).detail ?? null; }
  toJSON() { return detailJson(this, super.toJSON()); }
}

export class PerformanceMeasure extends PerformanceEntry {
  get detail() { return stateOf(this).detail ?? null; }
  toJSON() { return detailJson(this, super.toJSON()); }
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
  get(_type: unknown): undefined { return undefined; }
  has(_type: unknown): boolean { return false; }
  entries() { return [][Symbol.iterator](); }
  keys() { return [][Symbol.iterator](); }
  values() { return [][Symbol.iterator](); }
  forEach(_callback: unknown, _receiver?: unknown): void {}
  [Symbol.iterator]() { return this.entries(); }
}

export class PerformanceObserverEntryList {
  constructor() { throw new TypeError("Illegal constructor"); }
  getEntries() { return []; }
  getEntriesByName(_name: unknown, _type?: unknown): PerformanceEntry[] { return []; }
  getEntriesByType(_type: unknown): PerformanceEntry[] { return []; }
}

// Workerd accepts any provided callback value and does not deliver observations.
export class PerformanceObserver {
  #callback: unknown;
  static get supportedEntryTypes() { return ["measure", "mark"]; }
  constructor(callback: unknown) {
    if (arguments.length === 0) throw new TypeError("A callback argument is required");
    markHostObject(this);
    this.#callback = callback;
  }
  observe(_options?: unknown): void {}
  disconnect(): void {}
  takeRecords(): PerformanceEntry[] { return []; }
}

export class Performance extends EventTarget {
  #entries: PerformanceEntry[] = [];
  constructor(brand: unknown = undefined) {
    if (brand !== internal) throw new TypeError("Illegal constructor");
    super();
  }
  // Workerd uses epoch milliseconds, exactly as Date.now(), with timeOrigin 0.
  get timeOrigin() { return 0; }
  now() { return now(); }
  mark(name: unknown, options?: unknown): PerformanceMark {
    if (arguments.length === 0) throw new TypeError("A mark name is required");
    const mark = new PerformanceMark(name, options);
    this.#entries.push(mark);
    return mark;
  }
  measure(name: unknown, startOrOptions?: unknown, endMark?: unknown): PerformanceMeasure {
    if (arguments.length === 0) throw new TypeError("A measure name is required");
    let startTime = 0, endTime = now(), detail: unknown;
    if (isObject(startOrOptions)) {
      const { start, end, duration } = startOrOptions;
      detail = detailOption(startOrOptions);
      if (start !== undefined) startTime = Number(start);
      if (end !== undefined) endTime = Number(end);
      else if (duration !== undefined) endTime = startTime + Number(duration);
    } else if (startOrOptions !== undefined) {
      const markTime = (name: string) => this.#entries.map(stateOf).find(state => state.name === name && state.entryType === "mark")?.startTime;
      startTime = markTime(string(startOrOptions)) ?? 0;
      if (endMark !== undefined) endTime = markTime(string(endMark)) ?? endTime;
    }
    const measure = new PerformanceMeasure(internal, { name: string(name), entryType: "measure", startTime, duration: endTime >= startTime ? endTime - startTime : 0, detail });
    this.#entries.push(measure);
    return measure;
  }
  clearMarks(name?: unknown): void { if (name !== undefined) name = string(name); this.#entries = this.#entries.filter(entry => name === undefined ? stateOf(entry).entryType !== "mark" : stateOf(entry).name !== name); }
  clearMeasures(name?: unknown): void { if (name !== undefined) name = string(name); this.#entries = this.#entries.filter(entry => name === undefined ? stateOf(entry).entryType !== "measure" : stateOf(entry).name !== name); }
  clearResourceTimings(): void { this.#entries = this.#entries.filter(entry => !["resource", "navigation"].includes(stateOf(entry).entryType)); }
  getEntries(): PerformanceEntry[] { return [...this.#entries]; }
  getEntriesByName(name: unknown, type?: unknown): PerformanceEntry[] { name = string(name); if (type !== undefined) type = string(type); return this.#entries.filter(entry => { const state = stateOf(entry); return state.name === name && (type === undefined || state.entryType === type); }); }
  getEntriesByType(type: unknown): PerformanceEntry[] { type = string(type); return this.#entries.filter(entry => stateOf(entry).entryType === type); }
  get eventCounts() { return new EventCounts(); }
  eventLoopUtilization() { return { idle: 0, active: 0, utilization: 0 }; }
  get nodeTiming() { return new PerformanceNodeTiming(); }
  markResourceTiming() {}
  setResourceTimingBufferSize(_size: unknown): void {}
  timerify(callback: unknown): Function { if (typeof callback !== "function") throw new TypeError("The callback argument must be of type function"); return callback; }
  toJSON() { return json({ timeOrigin: 0 }); }
}

for (const Type of [PerformanceEntry, PerformanceMark, PerformanceMeasure, PerformanceResourceTiming, PerformanceNodeTiming, EventCounts, PerformanceObserverEntryList, PerformanceObserver, Performance]) {
  Object.defineProperty(Type.prototype, Symbol.toStringTag, { value: Type.name, configurable: true });
}

export const performance = new Performance(internal);
