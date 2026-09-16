const states = new WeakMap();
const trackerStates = new WeakMap();

function noOp() {}

function fail(type, message, code) {
  const error = new type(message);
  if (code) Object.defineProperty(error, "code", { value: code });
  throw error;
}

function validFunction(value, name) {
  if (typeof value !== "function") fail(TypeError, `${name} must be a function`, "ERR_INVALID_ARG_TYPE");
  return value;
}

function optionsValue(options) {
  if (options === undefined) return { times: Infinity };
  if (options === null || typeof options !== "object") fail(TypeError, "options must be an object", "ERR_INVALID_ARG_TYPE");
  const times = options.times === undefined ? Infinity : options.times;
  if (times === null || typeof times !== "number") fail(TypeError, "options.times must be an integer", "ERR_INVALID_ARG_TYPE");
  if (times !== Infinity && (!Number.isInteger(times) || times < 1)) {
    fail(RangeError, "options.times must be a positive integer", "ERR_OUT_OF_RANGE");
  }
  return { times };
}

function createState(original, implementation, options) {
  if (original === undefined) original = noOp;
  validFunction(original, "original");
  if (implementation === undefined) implementation = original;
  validFunction(implementation, "implementation");
  return {
    original,
    implementation,
    times: optionsValue(options).times,
    calls: [],
    once: new Map(),
    restoreTarget: null,
  };
}

function callImplementation(state, thisValue, args, target) {
  const call = {
    arguments: args,
    error: undefined,
    result: undefined,
    stack: new Error(),
    target,
    this: thisValue,
  };
  state.calls.push(call);
  const index = state.calls.length - 1;
  const implementation = state.once.has(index)
    ? state.once.get(index)
    : index < state.times ? state.implementation : state.original;
  try {
    if (target) {
      call.result = Reflect.construct(implementation, args, target);
      call.this = call.result;
    } else {
      call.result = Reflect.apply(implementation, thisValue, args);
    }
    return call.result;
  } catch (error) {
    call.error = error;
    throw error;
  }
}

function createMock(original, implementation, options, tracker) {
  if (implementation && typeof implementation === "object" && options === undefined) {
    options = implementation;
    implementation = undefined;
  }
  const state = createState(original, implementation, options);
  const context = new MockFunctionContext(state);
  const mocked = function (...args) {
    return callImplementation(state, this, args, new.target);
  };
  Object.defineProperty(mocked, "name", {
    configurable: true,
    value: original?.name || "mocked",
  });
  const proxy = new Proxy(mocked, {
    get(target, property, receiver) {
      if (property === "mock") return context;
      return Reflect.get(target, property, receiver);
    },
  });
  if (tracker) trackerStates.get(tracker).add(state);
  return proxy;
}

function findProperty(object, property) {
  for (let owner = object; owner; owner = Object.getPrototypeOf(owner)) {
    const descriptor = Object.getOwnPropertyDescriptor(owner, property);
    if (descriptor) return { descriptor, owner };
  }
  fail(TypeError, "property does not exist", "ERR_INVALID_ARG_VALUE");
}

function mockProperty(tracker, object, property, implementation, options, kind) {
  if (object === null || (typeof object !== "object" && typeof object !== "function")) {
    fail(TypeError, "object must be an object", "ERR_INVALID_ARG_TYPE");
  }
  if (implementation && typeof implementation === "object" && options === undefined) {
    options = implementation;
    implementation = undefined;
  }
  if (options === null || (options !== undefined && typeof options !== "object")) {
    fail(TypeError, "options must be an object", "ERR_INVALID_ARG_TYPE");
  }
  const found = findProperty(object, property);
  const descriptor = found.descriptor;
  const original = kind === "method" ? descriptor.value : descriptor[kind];
  if (typeof original !== "function") fail(TypeError, `${kind} target must be a function`, "ERR_INVALID_ARG_VALUE");
  const mock = createMock(original, implementation, options, tracker);
  const replacement = kind === "method"
    ? { ...descriptor, value: mock }
    : { ...descriptor, [kind]: mock };
  Object.defineProperty(object, property, replacement);
  states.get(mock.mock).restoreTarget = () => {
    if (found.owner === object) Object.defineProperty(object, property, descriptor);
    else delete object[property];
  };
  return mock;
}

function accessorOptions(options, kind) {
  if (options !== undefined && (options === null || typeof options !== "object")) {
    fail(TypeError, "options must be an object", "ERR_INVALID_ARG_TYPE");
  }
  return { ...options, [kind]: true };
}

function restoreState(state) {
  state.restoreTarget?.();
  state.implementation = state.original;
  state.times = Infinity;
  state.once.clear();
}

export class MockFunctionContext {
  constructor(state = null) {
    states.set(this, state ?? createState());
  }

  get calls() {
    return states.get(this).calls.slice();
  }

  callCount() {
    return states.get(this).calls.length;
  }

  mockImplementation(implementation) {
    const state = states.get(this);
    state.implementation = validFunction(implementation, "implementation");
    state.times = Infinity;
    state.once.clear();
  }

  mockImplementationOnce(implementation, onCall) {
    const state = states.get(this);
    validFunction(implementation, "implementation");
    const index = onCall ?? state.calls.length;
    if (!Number.isInteger(index) || index < state.calls.length) {
      fail(RangeError, "onCall must be a future invocation number", "ERR_OUT_OF_RANGE");
    }
    state.once.set(index, implementation);
  }

  restore() {
    const state = states.get(this);
    state.restoreTarget?.();
    state.implementation = state.original;
    state.times = Infinity;
    state.once.clear();
  }

  resetCalls() {
    states.get(this).calls.length = 0;
  }
}

export class MockTracker {
  constructor() {
    trackerStates.set(this, new Set());
  }

  fn(original, implementation, options) {
    return createMock(original, implementation, options, this);
  }

  method(object, property, implementation, options) {
    if (options?.getter || options?.setter) {
      if (options.getter && options.setter) {
        fail(TypeError, "getter and setter cannot both be true", "ERR_INVALID_ARG_VALUE");
      }
      return mockProperty(this, object, property, implementation, options, options.getter ? "get" : "set");
    }
    return mockProperty(this, object, property, implementation, options, "method");
  }

  getter(object, property, implementation, options) {
    if (implementation && typeof implementation === "object" && options === undefined) {
      options = implementation;
      implementation = undefined;
    }
    return mockProperty(this, object, property, implementation, accessorOptions(options, "getter"), "get");
  }

  setter(object, property, implementation, options) {
    if (implementation && typeof implementation === "object" && options === undefined) {
      options = implementation;
      implementation = undefined;
    }
    return mockProperty(this, object, property, implementation, accessorOptions(options, "setter"), "set");
  }

  reset() {
    const mocks = trackerStates.get(this);
    for (const state of mocks) restoreState(state);
    mocks.clear();
  }

  restoreAll() {
    for (const state of trackerStates.get(this)) restoreState(state);
  }
}

export const mock = new MockTracker();

export default { MockFunctionContext, MockTracker, mock };
