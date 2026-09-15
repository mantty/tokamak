import * as perf from "node:perf_hooks";

function outcome(callback) {
  try { return callback(); }
  catch (error) { return { error: error.name, code: error.code ?? null }; }
}

export async function performanceContracts() {
  const p = performance;
  p.clearMarks(); p.clearMeasures();
  const result = {
    clock: { origin: p.timeOrigin, epochBased: Math.abs(p.now() - Date.now()) < 100 },
    constructors: [() => new Performance(), () => new PerformanceEntry(), () => new PerformanceMeasure(), () => new PerformanceObserverEntryList()]
      .map(callback => outcome(callback)),
    marks: [-1, NaN, Infinity].map(startTime => outcome(() => new PerformanceMark("mark", { startTime }).toJSON())),
    nonFinite: [NaN, Infinity, -Infinity].map(startTime => {
      const mark = new PerformanceMark("non-finite", { startTime });
      return { nan: Number.isNaN(mark.startTime), positiveInfinity: mark.startTime === Infinity, negativeInfinity: mark.startTime === -Infinity };
    }),
    detailValidation: [1, "text", false, null, undefined, () => {}].map(detail => outcome(() => new PerformanceMark("detail", { startTime: 1, detail }).toJSON())),
  };
  p.mark("a", { startTime: 10 }); p.mark("b", { startTime: 2 });
  p.mark("a", { startTime: 20 });
  result.measure = [["m", "a", "b"], ["m", { start: 1, end: 5 }], ["m", { start: 1, duration: 4 }],
    ["m", { end: 5, duration: 4 }], ["m", { duration: 4 }], ["m", { start: 1, end: 5, duration: 2 }],
    ["m", { start: -1, end: 2 }], ["m", { start: "missing", end: 5 }]]
    .map(args => outcome(() => p.measure(...args).toJSON()));
  const detail = { value: 1 };
  const mark = p.mark("reference", { startTime: 3, detail }); detail.value = 2;
  result.detail = { same: mark.detail === detail, value: mark.detail, json: mark.toJSON(), nullPrototype: Object.getPrototypeOf(mark.toJSON()) === null };
  result.nodeTiming = outcome(() => ({ json: p.nodeTiming.toJSON(), own: Object.getOwnPropertyNames(p.nodeTiming).sort(),
    prototype: Object.getOwnPropertyNames(Object.getPrototypeOf(p.nodeTiming)).sort(), tag: Object.prototype.toString.call(p.nodeTiming), entry: p.nodeTiming instanceof PerformanceEntry }));
  result.events = outcome(() => ({ tag: Object.prototype.toString.call(p.eventCounts), size: p.eventCounts.size,
    entries: [...p.eventCounts], has: p.eventCounts.has("click"), value: p.eventCounts.get("click") }));
  const fn = function (a) { return [this.value, a]; };
  result.timerify = { identity: p.timerify(fn) === fn, nodeIdentity: perf.timerify(fn) === fn, call: p.timerify(fn).call({ value: 5 }, 7) };
  result.utilization = perf.eventLoopUtilization({ idle: 2, active: 3, utilization: 0.6 });
  result.eventsDispatch = outcome(() => {
    let called = 0;
    const listener = () => { called++; };
    p.addEventListener("custom", listener);
    const dispatched = p.dispatchEvent(new Event("custom"));
    p.removeEventListener("custom", listener);
    return { called, dispatched, target: p instanceof EventTarget };
  });
  result.startTimeGetter = outcome(() => {
    let calls = 0;
    const mark = p.mark("getter", { get startTime() { return ++calls; } });
    return { calls, start: mark.startTime };
  });
  result.nameConversion = [false, true].map(construct => {
    const options = { startTime: 1 };
    let calls = 0;
    const name = { toString() { calls++; options.startTime = 7; return "converted"; } };
    const mark = construct ? new PerformanceMark(name, options) : p.mark(name, options);
    return { calls, start: mark.startTime };
  });
  result.shadowedEntry = outcome(() => {
    const mark = p.mark("intrinsic", { startTime: 4 });
    Object.defineProperties(mark, { name: { value: "changed" }, startTime: { value: 99 }, entryType: { value: "changed" }, duration: { value: 99 } });
    const json = mark.toJSON();
    const found = p.getEntriesByName("intrinsic").includes(mark);
    const typed = p.getEntriesByType("mark").includes(mark);
    const measure = p.measure("using-intrinsic", "intrinsic", "intrinsic").toJSON();
    p.clearMarks("intrinsic");
    return { json, found, typed, measure, removed: !p.getEntries().includes(mark) };
  });
  result.observerArguments = [1, null, undefined, {}].map(value => outcome(() => { new PerformanceObserver(value); return true; }));
  result.observation = await (async () => {
    const calls = [];
    const observer = new PerformanceObserver(list => calls.push(list.getEntries()));
    observer.observe({ entryTypes: ["mark", "measure"], buffered: true });
    p.mark("observed", { startTime: 1 });
    await new Promise(resolve => setTimeout(resolve, 0));
    observer.disconnect();
    return { calls, records: observer.takeRecords() };
  })();
  result.json = p.toJSON();
  result.clearing = ["clearMarks", "clearMeasures"].flatMap(method => [false, true].map(named => {
    p.clearMarks(); p.clearMeasures();
    p.mark("shared", { startTime: 1 });
    p.measure("shared", { start: 1, end: 2 });
    p.mark("other", { startTime: 3 });
    p.measure("other", { start: 3, end: 4 });
    if (named) p[method]("shared"); else p[method]();
    return p.getEntries().map(entry => [entry.name, entry.entryType]);
  }));
  p.clearMarks(); p.clearMeasures();
  return result;
}
