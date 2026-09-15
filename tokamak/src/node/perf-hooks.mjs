import {
  Performance, PerformanceEntry, PerformanceMark, PerformanceMeasure, PerformanceObserver, PerformanceObserverEntryList,
  PerformanceResourceTiming, performance,
} from "../globals/web.mjs";

PerformanceObserver.supportedEntryTypes = ["measure", "mark"];

export function timerify(callback) {
  if (typeof callback !== "function") throw new TypeError("The callback argument must be of type function");
  return function (...args) { return callback.apply(this, args); };
}

export function eventLoopUtilization(utilization1, utilization2) {
  if (!utilization1) return { idle: 0, active: 0, utilization: 0 };
  if (!utilization2) return { idle: utilization1.idle ?? 0, active: utilization1.active ?? 0, utilization: utilization1.utilization ?? 0 };
  const idle = Math.max(0, (utilization1.idle ?? 0) - (utilization2.idle ?? 0));
  const active = Math.max(0, (utilization1.active ?? 0) - (utilization2.active ?? 0));
  return { idle, active, utilization: idle + active === 0 ? 0 : active / (idle + active) };
}

function unsupported(name) {
  const error = new Error(`node:perf_hooks ${name} is not implemented`);
  error.code = "ERR_METHOD_NOT_IMPLEMENTED";
  throw error;
}

export function createHistogram() { return unsupported("createHistogram"); }
export function monitorEventLoopDelay() { return unsupported("monitorEventLoopDelay"); }

export const constants = {
  NODE_PERFORMANCE_GC_MAJOR: 4, NODE_PERFORMANCE_GC_MINOR: 1, NODE_PERFORMANCE_GC_INCREMENTAL: 8, NODE_PERFORMANCE_GC_WEAKCB: 16,
  NODE_PERFORMANCE_GC_FLAGS_NO: 0, NODE_PERFORMANCE_GC_FLAGS_CONSTRUCT_RETAINED: 2, NODE_PERFORMANCE_GC_FLAGS_FORCED: 4,
  NODE_PERFORMANCE_GC_FLAGS_SYNCHRONOUS_PHANTOM_PROCESSING: 8, NODE_PERFORMANCE_GC_FLAGS_ALL_AVAILABLE_GARBAGE: 16,
  NODE_PERFORMANCE_GC_FLAGS_ALL_EXTERNAL_MEMORY: 32, NODE_PERFORMANCE_GC_FLAGS_SCHEDULE_IDLE: 64,
  NODE_PERFORMANCE_ENTRY_TYPE_GC: 0, NODE_PERFORMANCE_ENTRY_TYPE_HTTP: 1, NODE_PERFORMANCE_ENTRY_TYPE_HTTP2: 2,
  NODE_PERFORMANCE_ENTRY_TYPE_NET: 3, NODE_PERFORMANCE_ENTRY_TYPE_DNS: 4, NODE_PERFORMANCE_MILESTONE_TIME_ORIGIN_TIMESTAMP: 0,
  NODE_PERFORMANCE_MILESTONE_TIME_ORIGIN: 1, NODE_PERFORMANCE_MILESTONE_ENVIRONMENT: 2, NODE_PERFORMANCE_MILESTONE_NODE_START: 3,
  NODE_PERFORMANCE_MILESTONE_V8_START: 4, NODE_PERFORMANCE_MILESTONE_LOOP_START: 5, NODE_PERFORMANCE_MILESTONE_LOOP_EXIT: 6,
  NODE_PERFORMANCE_MILESTONE_BOOTSTRAP_COMPLETE: 7,
};

export {
  Performance, PerformanceEntry, PerformanceMark, PerformanceMeasure, PerformanceObserver, PerformanceObserverEntryList,
  PerformanceResourceTiming, performance,
};

export default {
  Performance, PerformanceEntry, PerformanceMark, PerformanceMeasure, PerformanceObserver, PerformanceObserverEntryList,
  PerformanceResourceTiming, performance, timerify, eventLoopUtilization, monitorEventLoopDelay, createHistogram, constants,
};
