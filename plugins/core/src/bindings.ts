import { nativeConnection, type TokamakContext } from "./context.js";
import { decodeJson, encodeJson } from "./codec.js";
import { nativeError, type NativeFailure } from "./native-protocol.mjs";
import { NativeReference, ownerOf } from "./resources.js";
import { byteStream } from "./streams.js";

export type { TokamakContext } from "./context.js";
export { NativeReference } from "./resources.js";

/** The reference `object` views: pass it to another class's constructor or to a plugin's native endpoint. */
export let referenceOf: (object: NativeObject) => NativeReference;

/**
 * A native object that a generated binding returned. Its class mirrors the native class; every
 * member is asynchronous. The invocation that received it owns it until `release`, the
 * invocation's end, or garbage collection.
 */
export class NativeObject {
  readonly #reference: NativeReference;

  static {
    referenceOf = (object) => object.#reference;
  }

  /** Views the object `reference` holds as this class. The native side checks the type at each call. */
  constructor(reference: NativeReference) {
    this.#reference = reference;
  }
}

/** Releases `object` for every view of it. Later calls on it reject with `InvalidStateError`. */
export function release(object: NativeObject): void {
  referenceOf(object).close();
}

type NativeClass = new (reference: NativeReference) => NativeObject;

/** The classes of a binding module, by native name. */
export type Classes = Readonly<Record<string, NativeClass>>;

/** The calls of one generated binding module, whose native plugin is `plugin`. */
export class Bindings {
  readonly #plugin: string;
  readonly #classes: () => Classes;

  /** `classes` returns the module's classes once they are defined. */
  constructor(plugin: string, classes: () => Classes) {
    this.#plugin = plugin;
    this.#classes = classes;
  }

  /**
   * Calls `member` of `target`, the object or, for a static member, the invocation's context.
   * Aborting `signal` cancels an asynchronous member's task.
   */
  async call<T>(target: TokamakContext | NativeObject, member: string, args: readonly unknown[], signal?: AbortSignal): Promise<T> {
    const ctx = contextOf(target);
    const reply = await ctx[nativeConnection].call(this.#plugin, member, await request(ctx, target, args), { signal });
    return this.#decode(ctx, reply) as T;
  }

  /**
   * The key of the first of `overloads` whose parameters take `args`: each parameter lists the
   * kinds of value it takes, a class or a `typeof` name, `"array"`, `"bytes"`, `"date"`, `"object"`
   * (a plain object), `"null"` or `"any"`.
   */
  overload(args: readonly unknown[], overloads: readonly (readonly [string, readonly (readonly Kind[])[]])[]): string {
    const found = overloads.find(([, params]) =>
      params.length === args.length && params.every((kinds, index) => kinds.some((kind) => takes(kind, args[index]))));
    if (!found) throw new TypeError("No overload takes these arguments");
    return found[0];
  }

  /** The elements of the native sequence `member` of `target` returns. Cancelling the stream ends it. */
  stream<T>(target: TokamakContext | NativeObject, member: string, args: readonly unknown[]): ReadableStream<T> {
    const ctx = contextOf(target);
    const elements = byteStream((next, error, close) => {
      let stop: (() => void) | undefined;
      let stopped = false;
      request(ctx, target, args).then((payload) => {
        if (stopped) return;
        stop = ctx[nativeConnection].listen(this.#plugin, member, payload, {
          next: (value, done) => { if (done) close(); else next(value); },
          error,
        });
      }, error);
      return () => {
        stopped = true;
        stop?.();
      };
    });
    return elements.pipeThrough(new TransformStream<Uint8Array, T>({
      transform: (element, controller) => { controller.enqueue(this.#decode(ctx, element) as T); },
    }));
  }

  /**
   * A native object implementing `protocol` whose `methods`, where `handlers` has them, call
   * `handlers` in order. Its platform defaults remain for the methods `handlers` lacks.
   */
  async implement<T extends NativeObject>(
    ctx: TokamakContext,
    Protocol: new (reference: NativeReference) => T,
    protocol: string,
    methods: readonly string[],
    handlers: object,
  ): Promise<T> {
    const handled = methods.filter((method) => typeof Reflect.get(handlers, method) === "function");
    const payload = encodeJson({ scope: await scopeOf(ctx), methods: handled });
    const reference = await NativeReference.acquire(ctx, this.#plugin, protocol, payload, (value) => {
      const { method, arguments: args } = decodeJson<{ method: string; arguments: Wire[] }>(value);
      const classes = this.#classes();
      try {
        const handler = Reflect.get(handlers, method) as (...values: unknown[]) => unknown;
        Promise.resolve(handler.apply(handlers, args.map((arg) => fromWire(arg, ctx, classes)))).catch((error: unknown) => {
          reportError(error);
        });
      } catch (error) {
        reportError(error);
      }
    });
    return new Protocol(reference);
  }

  #decode(ctx: TokamakContext, payload: Uint8Array): unknown {
    return fromWire(decodeJson<Wire>(payload), ctx, this.#classes());
  }
}

/** A kind of value an overload's parameter takes. */
type Kind = string | (abstract new (reference: NativeReference) => NativeObject);

/** The kinds named by strings other than `typeof` results. */
const named = new Map<string, (value: unknown) => boolean>([
  ["any", () => true],
  ["null", (value) => value === null || value === undefined],
  ["array", (value) => Array.isArray(value)],
  ["bytes", (value) => value instanceof Uint8Array],
  ["date", (value) => value instanceof Date],
  ["object", (value) => typeof value === "object" && value !== null && !Array.isArray(value) && !(value instanceof NativeObject)],
]);

function takes(kind: Kind, value: unknown): boolean {
  if (typeof kind === "function") return value instanceof kind;
  const takes = named.get(kind);
  return takes ? takes(value) : typeof value === kind;
}

/** The Android application `Context`. */
export function applicationContext(ctx: TokamakContext): Promise<NativeReference> {
  return root(ctx, "context");
}

/** The Android activity the app shows. Rejects with `NeedsUIError` while the app cannot show UI. */
export function foregroundActivity(ctx: TokamakContext): Promise<NativeReference> {
  return root(ctx, "activity");
}

/**
 * The Apple object a native screen is presented from: the top `UIViewController` on iOS, the
 * `NSWindow` on macOS. Rejects with `NeedsUIError` while the app cannot show UI.
 */
export function presenter(ctx: TokamakContext): Promise<NativeReference> {
  return root(ctx, "presenter");
}

async function root(ctx: TokamakContext, name: string): Promise<NativeReference> {
  const payload = encodeJson({ scope: await scopeOf(ctx), name });
  const id = decodeJson<string>(await ctx[nativeConnection].call("tokamak", "resource.root", payload));
  return NativeReference.scoped(ctx, id);
}

const scopes = new WeakMap<TokamakContext, Promise<string>>();

/** The scope that keeps the objects the invocation `ctx` receives until it ends. */
function scopeOf(ctx: TokamakContext): Promise<string> {
  let scope = scopes.get(ctx);
  if (!scope) {
    // The subscription stays open for the invocation, whose end releases the scope's objects.
    scope = new Promise((resolve, reject) => {
      ctx[nativeConnection].listen("tokamak", "resource.scope", encodeJson(null), {
        next: (value) => { resolve(decodeJson<string>(value)); },
        error: reject,
      });
    });
    scopes.set(ctx, scope);
  }
  return scope;
}

function contextOf(target: TokamakContext | NativeObject): TokamakContext {
  return target instanceof NativeObject ? ownerOf(referenceOf(target)) : target;
}

async function request(ctx: TokamakContext, target: TokamakContext | NativeObject, args: readonly unknown[]): Promise<Uint8Array> {
  return encodeJson({
    scope: await scopeOf(ctx),
    target: target instanceof NativeObject ? referenceOf(target).idFor(ctx) : undefined,
    arguments: args.map((arg) => toWire(arg, ctx)),
  });
}

/**
 * Binding values as JSON. Objects tagged `#` carry what JSON lacks: native objects, dates, bytes,
 * errors, non-finite numbers, and records whose own keys include `#`.
 */
type Wire = null | boolean | number | string | Wire[] | { [key: string]: Wire };

function toWire(value: unknown, ctx: TokamakContext): Wire {
  if (value === null || value === undefined) return null;
  if (typeof value === "number" && !Number.isFinite(value)) return { "#": "number", value: String(value) };
  if (typeof value !== "object") return value as Wire;
  return objectToWire(value, ctx);
}

function objectToWire(value: object, ctx: TokamakContext): Wire {
  const reference = value instanceof NativeObject ? referenceOf(value) : value;
  if (reference instanceof NativeReference) return { "#": "object", id: reference.idFor(ctx) };
  if (value instanceof Date) return { "#": "date", value: value.getTime() };
  if (value instanceof Uint8Array) return { "#": "bytes", value: toBase64(value) };
  if (Array.isArray(value)) return value.map((item) => toWire(item, ctx));
  const fields = Object.fromEntries(Object.entries(value).map(([key, item]) => [key, toWire(item, ctx)]));
  return "#" in fields ? { "#": "record", value: fields } : fields;
}

type Tagged = Record<string, Wire>;

/** The decoder of each tag. */
const tags = new Map<string, (value: Tagged, ctx: TokamakContext, classes: Classes) => unknown>([
  ["object", (value, ctx, classes) => {
    const Class = (value.types as string[]).map((type) => classes[type]).find(Boolean) ?? NativeObject;
    return new Class(NativeReference.scoped(ctx, value.id as string));
  }],
  ["number", (value) => Number(value.value)],
  ["date", (value) => new Date(value.value as number)],
  ["bytes", (value) => fromBase64(value.value as string)],
  ["error", (value) => nativeError(value as unknown as NativeFailure)],
  ["record", (value, ctx, classes) => fields(value.value as Tagged, ctx, classes)],
]);

function fromWire(value: Wire, ctx: TokamakContext, classes: Classes): unknown {
  if (Array.isArray(value)) return value.map((item) => fromWire(item, ctx, classes));
  if (value === null || typeof value !== "object") return value;
  const decode = typeof value["#"] === "string" ? tags.get(value["#"]) : undefined;
  return decode ? decode(value, ctx, classes) : fields(value, ctx, classes);
}

function fields(value: Record<string, Wire>, ctx: TokamakContext, classes: Classes): Record<string, unknown> {
  return Object.fromEntries(Object.entries(value).map(([key, item]) => [key, fromWire(item, ctx, classes)]));
}

function toBase64(bytes: Uint8Array): string {
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary);
}

function fromBase64(text: string): Uint8Array {
  return Uint8Array.from(atob(text), (char) => char.charCodeAt(0));
}
