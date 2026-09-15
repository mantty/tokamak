function abortError() {
  const error = new Error("The operation was aborted");
  error.name = "AbortError";
  error.code = "ABORT_ERR";
  return error;
}

function validateOptions(options) {
  if (options === null || (options !== undefined && typeof options !== "object")) throw new TypeError("The options argument must be an object");
  return options ?? {};
}

function timerPromise(delay = 0, value, options) {
  return new Promise((resolve, reject) => {
    let settings;
    try { settings = validateOptions(options); }
    catch (error) { reject(error); return; }
    if (settings.signal?.aborted) { reject(abortError()); return; }
    const id = setTimeout(() => resolve(value), delay);
    settings.signal?.addEventListener("abort", () => { clearTimeout(id); reject(abortError()); }, { once: true });
  });
}

export const setTimeout = (...args) => globalThis.setTimeout(...args);
export const clearTimeout = (...args) => globalThis.clearTimeout(...args);
export const setInterval = (...args) => globalThis.setInterval(...args);
export const clearInterval = (...args) => globalThis.clearInterval(...args);
export const setImmediate = (...args) => globalThis.setImmediate(...args);
export const clearImmediate = (...args) => globalThis.clearImmediate(...args);

export function active(timer) { return timer; }
export function enroll() { throw new Error("Not implemented. Please use setTimeout() instead."); }
export function unenroll() {}

async function* interval(delay = 0, value, options) {
  const settings = validateOptions(options);
  let stopped = false;
  let resolveNext;
  const queue = [];
  const id = setInterval(() => {
    if (resolveNext) { const resolve = resolveNext; resolveNext = undefined; resolve({ value, done: false }); }
    else queue.push(value);
  }, delay);
  const abort = () => {
    stopped = true;
    clearInterval(id);
    resolveNext?.(Promise.reject(abortError()));
    resolveNext = undefined;
  };
  settings.signal?.addEventListener("abort", abort, { once: true });
  try {
    while (!stopped) {
      if (queue.length) { yield queue.shift(); continue; }
      yield await new Promise((resolve, reject) => { resolveNext = result => result instanceof Promise ? result.then(resolve, reject) : resolve(result); });
    }
  } finally {
    clearInterval(id);
    settings.signal?.removeEventListener("abort", abort);
  }
}

export const scheduler = { wait: (delay, options) => timerPromise(delay, undefined, options) };
export const promises = {
  setTimeout: timerPromise,
  setImmediate: (value, options) => timerPromise(0, value, options),
  setInterval: interval,
  scheduler,
};

const promiseModuleFacade = { ...promises };
let promiseModuleRead = false;
const timers = { active, clearImmediate, clearInterval, clearTimeout, enroll, setImmediate, setInterval, setTimeout, unenroll };
Object.defineProperty(timers, "promises", {
  configurable: true,
  enumerable: true,
  get() {
    if (!promiseModuleRead) { promiseModuleRead = true; return promises; }
    return promiseModuleFacade;
  },
});

export default timers;
