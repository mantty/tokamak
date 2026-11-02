import { isFunction, isObject, propertyOf, type AnyFunction } from "../globals/objects.ts";
import { propertyKey } from "../globals/conversions.ts";

interface MockCall {
  arguments: unknown[];
  error: unknown;
  result: unknown;
  stack: Error;
  target: Function | undefined;
  this: unknown;
}

interface MockState {
  original: AnyFunction;
  implementation: AnyFunction;
  times: number;
  calls: MockCall[];
  once: Map<number, AnyFunction>;
  restoreTarget: (() => void) | null;
}

// Set by MockFunctionContext: a context for `state`.
let contextOf: (state: MockState) => MockFunctionContext;

function noOp() {}

function fail(type: new (message: string) => Error, message: string, code?: string): never {
  const error = new type(message);
  if (code) Object.defineProperty(error, "code", { value: code });
  throw error;
}

function validFunction(value: unknown, name: string): AnyFunction {
  if (!isFunction(value)) fail(TypeError, `${name} must be a function`, "ERR_INVALID_ARG_TYPE");
  return value;
}

function validateOptions(options: unknown): asserts options is object | undefined {
  if (options === null || (options !== undefined && typeof options !== "object")) {
    fail(TypeError, "options must be an object", "ERR_INVALID_ARG_TYPE");
  }
}

function splitOptions(implementation: unknown, options: unknown): { implementation: unknown; options: unknown } {
  if (implementation && typeof implementation === "object" && options === undefined) return { implementation: undefined, options: implementation };
  return { implementation, options };
}

function timesOption(options: unknown): number {
  if (options === undefined) return Infinity;
  validateOptions(options);
  const value = propertyOf(options, "times");
  const times = value === undefined ? Infinity : value;
  if (typeof times !== "number") fail(TypeError, "options.times must be an integer", "ERR_INVALID_ARG_TYPE");
  if (times !== Infinity && (!Number.isInteger(times) || times < 1)) {
    fail(RangeError, "options.times must be a positive integer", "ERR_OUT_OF_RANGE");
  }
  return times;
}

function createState(original: unknown, implementation: unknown, options: unknown): MockState {
  const originalFunction = validFunction(original === undefined ? noOp : original, "original");
  return {
    original: originalFunction,
    implementation: validFunction(implementation === undefined ? originalFunction : implementation, "implementation"),
    times: timesOption(options),
    calls: [],
    once: new Map(),
    restoreTarget: null,
  };
}

function callImplementation(state: MockState, thisValue: unknown, args: unknown[], target: Function | undefined): unknown {
  const call: MockCall = {
    arguments: args,
    error: undefined,
    result: undefined,
    stack: new Error(),
    target,
    this: thisValue,
  };
  state.calls.push(call);
  const index = state.calls.length - 1;
  const implementation = state.once.get(index) ?? (index < state.times ? state.implementation : state.original);
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

// A mock function for `state`, named after `original`.
function mockFunction(state: MockState, original: unknown): AnyFunction {
  const context = contextOf(state);
  const mocked = function (this: unknown, ...args: unknown[]) {
    return callImplementation(state, this, args, new.target);
  };
  Object.defineProperty(mocked, "name", {
    configurable: true,
    value: propertyOf(original, "name") || "mocked",
  });
  return new Proxy(mocked, {
    get(target, property, receiver): unknown {
      if (property === "mock") return context;
      return Reflect.get(target, property, receiver);
    },
  });
}

function findProperty(object: object, property: PropertyKey): { descriptor: PropertyDescriptor; owner: object } {
  for (let owner: object | null = object; owner; owner = Object.getPrototypeOf(owner)) {
    const descriptor = Object.getOwnPropertyDescriptor(owner, property);
    if (descriptor) return { descriptor, owner };
  }
  fail(TypeError, "property does not exist", "ERR_INVALID_ARG_VALUE");
}

function accessorOptions(options: unknown, kind: "getter" | "setter"): object {
  validateOptions(options);
  return { ...options, [kind]: true };
}

function restoreState(state: MockState): void {
  state.restoreTarget?.();
  state.implementation = state.original;
  state.times = Infinity;
  state.once.clear();
}

export class MockFunctionContext {
  #state = createState(undefined, undefined, undefined);

  get calls(): MockCall[] {
    return this.#state.calls.slice();
  }

  callCount(): number {
    return this.#state.calls.length;
  }

  mockImplementation(implementation: unknown): void {
    this.#state.implementation = validFunction(implementation, "implementation");
    this.#state.times = Infinity;
    this.#state.once.clear();
  }

  mockImplementationOnce(implementation: unknown, onCall?: unknown): void {
    const valid = validFunction(implementation, "implementation");
    const index = onCall ?? this.#state.calls.length;
    if (typeof index !== "number" || !Number.isInteger(index) || index < this.#state.calls.length) {
      fail(RangeError, "onCall must be a future invocation number", "ERR_OUT_OF_RANGE");
    }
    this.#state.once.set(index, valid);
  }

  restore(): void {
    restoreState(this.#state);
  }

  resetCalls(): void {
    this.#state.calls.length = 0;
  }

  static {
    contextOf = state => {
      const context = new MockFunctionContext();
      context.#state = state;
      return context;
    };
  }
}

export class MockTracker {
  readonly #mocks = new Set<MockState>();

  // A mock function for `state`, which `reset` and `restoreAll` restore.
  #track(state: MockState, original: unknown): AnyFunction {
    this.#mocks.add(state);
    return mockFunction(state, original);
  }

  #mockProperty(object: unknown, property: unknown, implementation: unknown, options: unknown, kind: "method" | "get" | "set"): AnyFunction {
    if (!isObject(object)) fail(TypeError, "object must be an object", "ERR_INVALID_ARG_TYPE");
    ({ implementation, options } = splitOptions(implementation, options));
    validateOptions(options);
    const key = propertyKey(property);
    const found = findProperty(object, key);
    const descriptor = found.descriptor;
    const original: unknown = kind === "method" ? descriptor.value : descriptor[kind];
    if (typeof original !== "function") fail(TypeError, `${kind} target must be a function`, "ERR_INVALID_ARG_VALUE");
    const state = createState(original, implementation, options);
    const mock = this.#track(state, original);
    const replacement = kind === "method"
      ? { ...descriptor, value: mock }
      : { ...descriptor, [kind]: mock };
    Object.defineProperty(object, key, replacement);
    state.restoreTarget = () => {
      if (found.owner === object) Object.defineProperty(object, key, descriptor);
      else delete object[key];
    };
    return mock;
  }

  fn(original?: unknown, implementation?: unknown, options?: unknown): AnyFunction {
    ({ implementation, options } = splitOptions(implementation, options));
    return this.#track(createState(original, implementation, options), original);
  }

  method(object: unknown, property: unknown, implementation?: unknown, options?: unknown): AnyFunction {
    const getter = propertyOf(options, "getter"), setter = propertyOf(options, "setter");
    if (getter || setter) {
      if (getter && setter) {
        fail(TypeError, "getter and setter cannot both be true", "ERR_INVALID_ARG_VALUE");
      }
      return this.#mockProperty(object, property, implementation, options, getter ? "get" : "set");
    }
    return this.#mockProperty(object, property, implementation, options, "method");
  }

  getter(object: unknown, property: unknown, implementation?: unknown, options?: unknown): AnyFunction {
    ({ implementation, options } = splitOptions(implementation, options));
    return this.#mockProperty(object, property, implementation, accessorOptions(options, "getter"), "get");
  }

  setter(object: unknown, property: unknown, implementation?: unknown, options?: unknown): AnyFunction {
    ({ implementation, options } = splitOptions(implementation, options));
    return this.#mockProperty(object, property, implementation, accessorOptions(options, "setter"), "set");
  }

  reset(): void {
    for (const state of this.#mocks) restoreState(state);
    this.#mocks.clear();
  }

  restoreAll(): void {
    for (const state of this.#mocks) restoreState(state);
  }
}

export const mock = new MockTracker();

export default { MockFunctionContext, MockTracker, mock };
