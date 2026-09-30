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
    const id = globalThis.setTimeout(() => resolve(value), delay);
    settings.signal?.addEventListener("abort", () => { globalThis.clearTimeout(id); reject(abortError()); }, { once: true });
  });
}

export function setTimeout(delay, value, options) { return timerPromise(delay, value, options); }
export function setImmediate(value, options) { return timerPromise(0, value, options); }

export async function* setInterval(delay = 0, value, options) {
  const settings = validateOptions(options);
  let stopped = false;
  let resolveNext;
  const queue = [];
  const id = globalThis.setInterval(() => {
    if (resolveNext) { const resolve = resolveNext; resolveNext = undefined; resolve(value); }
    else queue.push(value);
  }, delay);
  const abort = () => {
    stopped = true;
    globalThis.clearInterval(id);
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
    globalThis.clearInterval(id);
    settings.signal?.removeEventListener("abort", abort);
  }
}

export const scheduler = { wait: (delay, options) => timerPromise(delay, undefined, options) };

export default { scheduler, setImmediate, setInterval, setTimeout };
