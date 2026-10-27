import events, { EventEmitter, EventEmitterAsyncResource } from "node:events";

// The name, code and message of a thrown error.
function failure(error) {
  return { error: error.name, code: error.code ?? null, message: error.message };
}

function attempt(callback) {
  try { return callback(); }
  catch (error) { return failure(error); }
}

async function asyncAttempt(callback) {
  try { return await callback(); }
  catch (error) { return failure(error); }
}

const turn = () => new Promise(resolve => setTimeout(resolve, 1));

// An emitter's own state: its property names and their values.
function ownState(emitter) {
  return {
    names: Object.getOwnPropertyNames(emitter),
    keys: Object.keys(emitter),
    events: Object.keys(emitter._events ?? {}),
    eventsPrototype: emitter._events && Object.getPrototypeOf(emitter._events),
    eventsCount: emitter._eventsCount,
    maxListeners: emitter._maxListeners ?? "undefined",
    domain: emitter.domain,
  };
}

function shapeContracts() {
  const prototype = EventEmitter.prototype;
  return {
    fresh: ownState(new EventEmitter()),
    listening: (() => {
      const emitter = new EventEmitter();
      const listener = () => {};
      emitter.on("a", listener);
      emitter.on("a", () => {});
      emitter.on(Symbol.for("b"), listener);
      return { ...ownState(emitter), single: typeof emitter._events[Symbol.for("b")], list: Array.isArray(emitter._events.a) };
    })(),
    afterRemoval: (() => {
      const emitter = new EventEmitter();
      const listener = () => {};
      emitter.on("a", listener);
      emitter.removeListener("a", listener);
      return ownState(emitter);
    })(),
    prototypeNames: Object.getOwnPropertyNames(prototype).sort(),
    prototypeValues: { events: prototype._events ?? "undefined", count: prototype._eventsCount, max: prototype._maxListeners ?? "undefined" },
    methods: Object.fromEntries(["emit", "on", "addListener", "once", "off", "removeListener", "prependListener", "listeners", "getMaxListeners"]
      .map(name => [name, [prototype[name].name, prototype[name].length]])),
    aliases: { on: prototype.on === prototype.addListener, off: prototype.off === prototype.removeListener },
    statics: Object.keys(EventEmitter).sort(),
    staticNames: { getMaxListeners: EventEmitter.getMaxListeners.name, listenerCount: EventEmitter.listenerCount.name, init: typeof EventEmitter.init },
    exports: Object.keys(events).sort(),
    captureSymbol: EventEmitter.captureRejectionSymbol === Symbol.for("nodejs.rejection"),
    errorMonitor: String(EventEmitter.errorMonitor),
    defaultMaxListeners: events.defaultMaxListeners,
  };
}

function listenerContracts() {
  const calls = [];
  class Recording extends EventEmitter {
    on(name, listener) { calls.push(["on", name]); return super.on(name, listener); }
    prependListener(name, listener) { calls.push(["prependListener", name]); return super.prependListener(name, listener); }
  }
  const recording = new Recording();
  recording.once("a", () => {});
  recording.prependOnceListener("b", () => {});
  const order = [];
  const emitter = new EventEmitter();
  const original = () => order.push("original");
  emitter.on("newListener", (name, listener) => order.push(["newListener", name, listener === original]));
  emitter.on("removeListener", (name, listener) => order.push(["removeListener", name, listener === original]));
  emitter.once("x", original);
  const raw = emitter.rawListeners("x")[0];
  const unwrapped = emitter.listeners("x")[0] === original;
  emitter.emit("x");
  emitter.emit("x");
  const duplicates = new EventEmitter();
  const marks = [];
  const repeated = () => marks.push("repeated");
  duplicates.on("y", repeated);
  duplicates.on("y", () => marks.push("other"));
  duplicates.on("y", repeated);
  duplicates.removeListener("y", repeated);
  duplicates.emit("y");
  const removing = new EventEmitter();
  const removed = [];
  removing.on("removeListener", name => removed.push(String(name)));
  removing.on("p", () => {});
  removing.on("q", () => {});
  removing.on("q", () => {});
  removing.removeAllListeners();
  const during = new EventEmitter();
  const seen = [];
  during.on("z", () => { seen.push(1); during.on("z", () => seen.push(3)); });
  during.on("z", () => seen.push(2));
  during.emit("z");
  return {
    calls,
    order,
    raw: { wrapped: raw !== original, listener: raw.listener === original, name: raw.name, unwrapped },
    duplicates: { marks, count: duplicates.listenerCount("y") },
    removed,
    removingState: ownState(removing),
    during: seen,
    names: (() => {
      const named = new EventEmitter();
      named.on("b", () => {});
      named.on(Symbol.for("s"), () => {});
      named.on("a", () => {});
      return named.eventNames().map(String);
    })(),
    emitResult: [new EventEmitter().emit("none"), (() => { const one = new EventEmitter(); one.on("v", () => {}); return one.emit("v"); })()],
    notFunction: [attempt(() => new EventEmitter().on("a", 5)), attempt(() => new EventEmitter().once("a", "x")), attempt(() => new EventEmitter().removeListener("a", null))],
  };
}

function errorContracts() {
  const monitored = [];
  const monitor = new EventEmitter();
  monitor.on(EventEmitter.errorMonitor, error => monitored.push(error.message));
  const unhandledObject = attempt(() => new EventEmitter().emit("error", { reason: 1 }));
  let context;
  try { new EventEmitter().emit("error", "text"); } catch (error) { context = error.context; }
  return {
    thrownError: attempt(() => new EventEmitter().emit("error", new TypeError("boom"))),
    unhandledObject,
    unhandledString: attempt(() => new EventEmitter().emit("error", "text")),
    unhandledNone: attempt(() => new EventEmitter().emit("error")),
    context,
    monitored: [attempt(() => monitor.emit("error", new Error("seen"))), monitored],
    maxListeners: [attempt(() => new EventEmitter().setMaxListeners(-1)), attempt(() => new EventEmitter().setMaxListeners("3")), attempt(() => new EventEmitter().setMaxListeners(NaN))],
    defaultMaxListeners: attempt(() => { EventEmitter.defaultMaxListeners = -1; }),
    staticSetMaxListeners: [attempt(() => events.setMaxListeners(-1)), attempt(() => events.setMaxListeners(1, {}))],
    captureRejections: [attempt(() => { EventEmitter.captureRejections = "yes"; }), attempt(() => new EventEmitter({ captureRejections: 1 }))],
    // Only the class: the message is the engine's own wording.
    bare: attempt(() => EventEmitter()).error,
  };
}

async function warningContracts() {
  const logged = [];
  const warnings = [];
  const log = console.log;
  const onWarning = warning => warnings.push({ name: warning.name, message: warning.message, type: warning.type, count: warning.count });
  process.on("warning", onWarning);
  console.log = (...args) => logged.push(args.map(String));
  try {
    const emitter = new EventEmitter();
    emitter.setMaxListeners(1);
    emitter.on("leak", () => {});
    emitter.on("leak", () => {});
    emitter.on("leak", () => {});
    const unlimited = new EventEmitter();
    unlimited.setMaxListeners(0);
    for (let index = 0; index < 20; index++) unlimited.on("many", () => {});
    await turn();
    return { logged, warnings, warned: emitter._events.leak.warned, max: emitter.getMaxListeners() };
  } finally {
    console.log = log;
    process.off("warning", onWarning);
  }
}

async function rejectionContracts() {
  const emitted = [];
  const capturing = new EventEmitter({ captureRejections: true });
  capturing.on("error", error => emitted.push(["error", error.message]));
  capturing.on("work", async () => { throw new Error("rejected"); });
  capturing.emit("work");
  const handled = [];
  const handler = new EventEmitter({ captureRejections: true });
  handler[Symbol.for("nodejs.rejection")] = (error, name, ...args) => handled.push([error.message, name, args]);
  handler.on("job", async () => { throw new Error("handled"); });
  handler.emit("job", 1, 2);
  const ignored = new EventEmitter();
  ignored.on("work", async () => { throw new Error("ignored"); });
  ignored.emit("work");
  const unhandled = await new Promise(resolve => {
    const listener = event => { event.preventDefault(); resolve(event.reason?.message); };
    globalThis.addEventListener("unhandledrejection", listener, { once: true });
    setTimeout(() => { globalThis.removeEventListener("unhandledrejection", listener); resolve("none"); }, 50);
  });
  await turn();
  return { emitted, handled, unhandled, capture: [capturing[Symbol.for("nodejs.rejection")] === undefined, EventEmitter.captureRejections] };
}

async function helperContracts() {
  const emitter = new EventEmitter();
  setTimeout(() => emitter.emit("ready", 1, 2), 1);
  const resolved = await events.once(emitter, "ready");
  const failing = new EventEmitter();
  setTimeout(() => failing.emit("error", new Error("failed")), 1);
  const rejected = await asyncAttempt(() => events.once(failing, "ready"));
  const aborted = AbortSignal.abort("why");
  const abortedOnce = await events.once(emitter, "x", { signal: aborted }).catch(error => ({ ...failure(error), cause: error.cause }));
  const controller = new AbortController();
  const pending = events.once(emitter, "never", { signal: controller.signal }).catch(error => ({ ...failure(error), cause: error.cause ?? null }));
  controller.abort();
  const lateAbort = await pending;
  const target = new EventTarget();
  setTimeout(() => target.dispatchEvent(new Event("ping")), 1);
  const [event] = await events.once(target, "ping");
  const iterated = [];
  const source = new EventEmitter();
  const iterator = events.on(source, "item");
  source.emit("item", "a");
  source.emit("item", "b", "c");
  for await (const args of iterator) {
    iterated.push(args);
    if (iterated.length === 2) break;
  }
  const iteratorShape = {
    prototype: Object.getPrototypeOf(iterator) === Object.getPrototypeOf(Object.getPrototypeOf(async function* () {}).prototype),
    keys: Object.keys(iterator),
    listeners: source.listenerCount("item"),
  };
  const erroring = new EventEmitter();
  const errorIterator = events.on(erroring, "item");
  erroring.emit("error", new Error("stream failed"));
  const iteratorError = await asyncAttempt(() => errorIterator.next());
  const afterError = await errorIterator.next();
  const abortedOn = attempt(() => events.on(source, "item", { signal: AbortSignal.abort() }));
  let abortCalls = 0;
  const disposable = events.addAbortListener(AbortSignal.abort(), () => { abortCalls++; });
  const before = abortCalls;
  await turn();
  return {
    resolved, rejected, abortedOnce, lateAbort, event: event.type, iterated, iteratorShape, iteratorError, afterError, abortedOn,
    abortListener: { before, after: abortCalls, prototype: Object.getPrototypeOf(disposable), dispose: typeof disposable[Symbol.dispose] },
    abortListenerErrors: [attempt(() => events.addAbortListener(undefined, () => {})), attempt(() => events.addAbortListener(new AbortController().signal, 5))],
    onceOptions: await asyncAttempt(() => events.once(emitter, "x", null)),
    getEventListeners: [events.getEventListeners(target, "ping"), events.getEventListeners(emitter, "ready").length, attempt(() => events.getEventListeners({}, "x"))],
    listenerCount: [events.listenerCount(emitter, "ready"), events.listenerCount({ _events: { a: [1, 2] } }, "a")],
  };
}

function asyncResourceContracts() {
  const resource = new EventEmitterAsyncResource({ name: "named" });
  return {
    names: Object.getOwnPropertyNames(resource),
    emitWithout: resource.emit("none"),
    resource: [typeof resource.asyncResource, resource.asyncResource.eventEmitter === resource, typeof resource.asyncResource.asyncId],
    prototype: Object.getOwnPropertyNames(EventEmitterAsyncResource.prototype).sort(),
  };
}

export async function eventsContracts() {
  return {
    shape: shapeContracts(),
    listeners: listenerContracts(),
    errors: errorContracts(),
    warnings: await warningContracts(),
    rejections: await rejectionContracts(),
    helpers: await helperContracts(),
    asyncResource: asyncResourceContracts(),
  };
}
