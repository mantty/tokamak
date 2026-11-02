import {
  htmlRewrite, htmlValidateSelector,
  type ElementAttribute, type HandlerSpec, type HtmlContent, type HtmlOperation, type HtmlOperationName, type TokenProperties,
} from "tokamak:host";
import { ReadableStream, drainedChunkBytes, nativeReadableStream, nativeStreamError } from "../streams/web.ts";
import { captureAsyncContext, runInAsyncContext } from "../builtins/async-context.ts";
import { string } from "../globals/conversions.ts";
import { isFunction, isObject, propertyOf, type AnyFunction } from "../globals/objects.ts";
import { Response } from "./fetch.ts";

// The properties the parser reports for one kind of token.
type Properties<T extends TokenProperties["token"]> = Extract<TokenProperties, { token: T }>;

// Applies an operation to the current token, returning its properties after.
type Mutate = (operation: HtmlOperation) => TokenProperties;

// Runs a handler on the token `properties` describe.
type TokenCallback = (properties: TokenProperties, resources: Resources) => Promise<void>;

// The kinds of handler a registration names.
type HandlerName = Exclude<keyof HandlerSpec, "selector">;

// A registration's handlers, by kind.
type Handlers = [HandlerName, TokenCallback][];

// The handlers and input sources of one transform, which the parser names by ID.
interface Resources {
  mutate: Mutate;
  retire(): void;
  addHandler(callback: TokenCallback, once?: boolean): number;
  addSource(stream: ReadableStream): number;
}

// An input stream the parser reads: its reader, the chunk being read, and
// whether the parser has started reading it.
interface Source {
  reader: ReadableStreamDefaultReader;
  bytes: Uint8Array | null;
  offset: number;
  started: boolean;
  pending: Promise<ReadableStreamReadResult<unknown>> | null;
}

// A token's state: how it changes and, when it takes streamed content, the
// transform's resources.
interface TokenState {
  mutate: Mutate;
  resources?: Resources;
}

interface ElementState extends TokenState {
  attributes: ElementAttribute[];
  attributeVersion: number;
  namespaceURI: string;
  resources: Resources;
  removed: boolean;
  tagName: string;
}

interface TextState extends TokenState {
  lastInTextNode: boolean;
  removed: boolean;
  text: string;
}

interface CommentState extends TokenState {
  removed: boolean;
  text: string;
}

interface EndTagState extends TokenState {
  name: string;
}

interface AttributesState {
  owner: Element;
  entries: [string, string][];
  index: number;
  version: number;
}

interface RewriterState {
  handlers: { selector: string; handlers: Handlers }[];
  documentHandlers: Handlers[];
}

const elementStates = new WeakMap<object, ElementState>();
const textStates = new WeakMap<object, TextState>();
const commentStates = new WeakMap<object, CommentState>();
const endTagStates = new WeakMap<object, EndTagState>();
const documentEndStates = new WeakMap<object, TokenState>();
const attributesStates = new WeakMap<object, AttributesState>();
const rewriterStates = new WeakMap<object, RewriterState>();
const doctypeStates = new WeakMap<object, Properties<"doctype">>();
const expiredTokens = new WeakSet<object>();

function contentOperation<T>(states: WeakMap<object, TokenState>, token: T, name: HtmlOperationName, value: unknown, options: unknown): T {
  const state = requireState(states, token);
  const contentType = propertyOf(options, "html") === true ? "html" : "text";
  let content: HtmlContent;
  if (value instanceof ReadableStream || value instanceof Response) {
    if (!state.resources) throw new TypeError("This HTML token requires string content");
    const stream = value instanceof Response ? value.body : value;
    content = stream === null ? "" : { source: state.resources.addSource(stream) };
  } else content = string(value);
  applyMutation(state, { name, value: content, contentType });
  return token;
}

function requireState<S>(states: WeakMap<object, S>, value: unknown): S {
  if (!isObject(value)) throw new TypeError("Illegal invocation");
  const state = states.get(value);
  if (state === undefined) throw new TypeError("Illegal invocation");
  if (expiredTokens.has(value)) throw new TypeError("This content token is no longer valid. Content tokens are only valid during the execution of the relevant content handler.");
  return state;
}

function applyMutation(state: TokenState, operation: HtmlOperation): void {
  Object.assign(state, state.mutate(operation));
}

// Applies an operation that changes an element's attributes, which ends
// iterations of the attributes before it.
function mutateAttributes(state: ElementState, operation: HtmlOperation): void {
  applyMutation(state, operation);
  state.attributeVersion++;
}

function asciiLower(value: unknown): string { return String(value).replace(/[A-Z]/g, letter => letter.toLowerCase()); }

class AttributesIterator {
  constructor(owner: Element, entries: [string, string][], version: number) {
    attributesStates.set(this, { owner, entries, index: 0, version });
  }

  next(): IteratorResult<[string, string], undefined> {
    const state = requireState(attributesStates, this);
    const element = requireState(elementStates, state.owner);
    if (state.version !== element.attributeVersion) throw new Error("The attributes have been modified during iteration");
    if (state.index === state.entries.length) return { value: undefined, done: true };
    return { value: state.entries[state.index++], done: false };
  }

  [Symbol.iterator](): this { return this; }

  get [Symbol.toStringTag](): string { return "AttributesIterator"; }
}

class Element {
  constructor(properties: Properties<"element">, mutate: Mutate, resources: Resources) {
    const state: ElementState = {
      attributes: properties.attributes.map(({ name, value }) => ({ name: asciiLower(name), value })),
      attributeVersion: 0,
      namespaceURI: properties.namespaceURI,
      mutate,
      resources,
      removed: properties.removed,
      tagName: properties.tagName,
    };
    elementStates.set(this, state);
    Object.defineProperties(this, {
      tagName: {
        enumerable: true,
        get() { return requireState(elementStates, this).tagName; },
        set(value: unknown) { applyMutation(requireState(elementStates, this), { name: "setTagName", value: String(value) }); },
      },
      namespaceURI: { enumerable: true, get() { return requireState(elementStates, this).namespaceURI; } },
      attributes: {
        enumerable: true,
        get() {
          const state = requireState(elementStates, this);
          const entries = state.attributes.map(({ name, value }): [string, string] => [name, value]);
          return new AttributesIterator(this, entries, state.attributeVersion);
        },
      },
      removed: { enumerable: true, get() { return requireState(elementStates, this).removed; } },
    });
  }

  getAttribute(name: unknown): string | null {
    const state = requireState(elementStates, this);
    const key = asciiLower(name);
    return state.attributes.find(attribute => attribute.name === key)?.value ?? null;
  }

  hasAttribute(name: unknown): boolean {
    const state = requireState(elementStates, this);
    const key = asciiLower(name);
    return state.attributes.some(attribute => attribute.name === key);
  }

  setAttribute(name: unknown, value: unknown): this {
    mutateAttributes(requireState(elementStates, this), { name: "setAttribute", attribute: asciiLower(name), value: String(value) });
    return this;
  }

  removeAttribute(name: unknown): this {
    mutateAttributes(requireState(elementStates, this), { name: "removeAttribute", attribute: asciiLower(name) });
    return this;
  }

  before(value: unknown, options?: unknown): this { return contentOperation(elementStates, this, "before", value, options); }
  after(value: unknown, options?: unknown): this { return contentOperation(elementStates, this, "after", value, options); }
  prepend(value: unknown, options?: unknown): this { return contentOperation(elementStates, this, "prepend", value, options); }
  append(value: unknown, options?: unknown): this { return contentOperation(elementStates, this, "append", value, options); }
  replace(value: unknown, options?: unknown): this { return contentOperation(elementStates, this, "replace", value, options); }
  setInnerContent(value: unknown, options?: unknown): this { return contentOperation(elementStates, this, "setInnerContent", value, options); }

  remove(): this {
    applyMutation(requireState(elementStates, this), { name: "remove" });
    return this;
  }

  removeAndKeepContent(): this {
    applyMutation(requireState(elementStates, this), { name: "removeAndKeepContent" });
    return this;
  }

  onEndTag(handler: unknown): void {
    if (!isFunction(handler)) throw new TypeError("HTMLRewriter end-tag handler must be a function");
    const state = requireState(elementStates, this);
    applyMutation(state, { name: "onEndTag", handler: state.resources.addHandler(tokenHandler(handler, undefined), true) });
  }

  get [Symbol.toStringTag](): string { return "Element"; }
}

class Text {
  constructor(properties: Properties<"text">, mutate: Mutate, resources: Resources) {
    const state: TextState = {
      lastInTextNode: properties.lastInTextNode,
      mutate,
      resources,
      removed: properties.removed,
      text: properties.text,
    };
    textStates.set(this, state);
    Object.defineProperties(this, {
      text: { enumerable: true, get() { return requireState(textStates, this).text; } },
      lastInTextNode: { enumerable: true, get() { return requireState(textStates, this).lastInTextNode; } },
      removed: { enumerable: true, get() { return requireState(textStates, this).removed; } },
    });
  }

  before(value: unknown, options?: unknown): this { return contentOperation(textStates, this, "before", value, options); }
  after(value: unknown, options?: unknown): this { return contentOperation(textStates, this, "after", value, options); }
  replace(value: unknown, options?: unknown): this { return contentOperation(textStates, this, "replace", value, options); }

  remove(): this {
    applyMutation(requireState(textStates, this), { name: "remove" });
    return this;
  }

  get [Symbol.toStringTag](): string { return "Text"; }
}

class Comment {
  constructor(properties: Properties<"comment">, mutate: Mutate) {
    commentStates.set(this, { mutate, removed: properties.removed, text: properties.text });
    Object.defineProperties(this, {
      text: {
        enumerable: true,
        get() { return requireState(commentStates, this).text; },
        set(value: unknown) { applyMutation(requireState(commentStates, this), { name: "setText", value: String(value) }); },
      },
      removed: { enumerable: true, get() { return requireState(commentStates, this).removed; } },
    });
  }

  before(value: unknown, options?: unknown): this { return contentOperation(commentStates, this, "before", value, options); }
  after(value: unknown, options?: unknown): this { return contentOperation(commentStates, this, "after", value, options); }
  replace(value: unknown, options?: unknown): this { return contentOperation(commentStates, this, "replace", value, options); }

  remove(): this {
    applyMutation(requireState(commentStates, this), { name: "remove" });
    return this;
  }

  get [Symbol.toStringTag](): string { return "Comment"; }
}

class EndTag {
  constructor(properties: Properties<"endTag">, mutate: Mutate, resources: Resources) {
    endTagStates.set(this, { mutate, resources, name: properties.name });
    Object.defineProperties(this, {
      name: { enumerable: true, get() { return requireState(endTagStates, this).name; } },
    });
  }

  before(value: unknown, options?: unknown): this { return contentOperation(endTagStates, this, "before", value, options); }
  after(value: unknown, options?: unknown): this { return contentOperation(endTagStates, this, "after", value, options); }

  remove(): this {
    applyMutation(requireState(endTagStates, this), { name: "remove" });
    return this;
  }

  get [Symbol.toStringTag](): string { return "EndTag"; }
}

class Doctype {
  constructor(properties: Properties<"doctype">) {
    doctypeStates.set(this, properties);
    Object.defineProperties(this, {
      name: { enumerable: true, get() { return requireState(doctypeStates, this).name ?? null; } },
      publicId: { enumerable: true, get() { return requireState(doctypeStates, this).publicId ?? null; } },
      systemId: { enumerable: true, get() { return requireState(doctypeStates, this).systemId ?? null; } },
    });
  }

  get [Symbol.toStringTag](): string { return "Doctype"; }
}

class DocumentEnd {
  constructor(mutate: Mutate) {
    documentEndStates.set(this, { mutate });
  }

  append(value: unknown, options?: unknown): this { return contentOperation(documentEndStates, this, "append", value, options); }

  get [Symbol.toStringTag](): string { return "DocumentEnd"; }
}

// The token `properties` describe, for one call of its handler.
function createToken(properties: TokenProperties, resources: Resources): object {
  switch (properties.token) {
    case "element": return new Element(properties, resources.mutate, resources);
    case "text": return new Text(properties, resources.mutate, resources);
    case "comment": return new Comment(properties, resources.mutate);
    case "doctype": return new Doctype(properties);
    case "endTag": return new EndTag(properties, resources.mutate, resources);
    case "end": return new DocumentEnd(resources.mutate);
  }
}

function callbackHandler(registered: unknown, name: HandlerName): TokenCallback | undefined {
  const callback = propertyOf(registered, name);
  if (callback === undefined) return undefined;
  if (!isFunction(callback)) throw new TypeError(`HTMLRewriter ${name} handler must be a function`);
  return tokenHandler(callback, registered);
}

// Calls `callback` on `receiver` with a token that expires when the callback settles.
function tokenHandler(callback: AnyFunction, receiver: unknown): TokenCallback {
  return async (properties, resources) => {
    const token = createToken(properties, resources);
    try { await callback.call(receiver, token); }
    finally { expiredTokens.add(token); }
  };
}

function captureHandlers(registered: unknown, names: HandlerName[]): Handlers {
  const handlers: Handlers = [];
  for (const name of names) {
    const handler = callbackHandler(registered, name);
    if (handler !== undefined) handlers.push([name, handler]);
  }
  return handlers;
}

// The entry `id` of `entries`, which the parser names only while it is registered.
function registered<T>(entries: Map<number, T>, id: number): T {
  const entry = entries.get(id);
  if (entry === undefined) throw new TypeError(`HTMLRewriter has no registration ${id}`);
  return entry;
}

function openSource(stream: ReadableStream): Source {
  return { reader: stream.getReader(), bytes: null, offset: 0, started: false, pending: null };
}

function responseInit(response: Response): object {
  return {
    status: response.status,
    statusText: response.statusText,
    headers: response.headers,
    url: response.url,
    redirected: response.redirected,
    type: response.type,
    cf: response.cf,
    webSocket: response.webSocket,
  };
}

export class HTMLRewriter {
  constructor() {
    rewriterStates.set(this, { handlers: [], documentHandlers: [] });
  }

  on(selector: unknown, handlers: unknown): this {
    if (arguments.length < 2) throw new TypeError("Invalid HTMLRewriter handler");
    const pattern = `${selector}`;
    htmlValidateSelector(pattern);
    requireState(rewriterStates, this).handlers.push({
      selector: pattern,
      handlers: captureHandlers(handlers, ["element", "text", "comments"]),
    });
    return this;
  }

  onDocument(handlers: unknown): this {
    if (arguments.length < 1) throw new TypeError("Invalid HTMLRewriter document handler");
    requireState(rewriterStates, this).documentHandlers.push(
      captureHandlers(handlers ?? {}, ["doctype", "comments", "text", "end"]),
    );
    return this;
  }

  transform(response: unknown): Response {
    const state = requireState(rewriterStates, this);
    if (!(response instanceof Response)) throw new TypeError("HTMLRewriter.transform requires a Response");
    if (response.bodyUsed || response.body?.locked) throw new TypeError("HTMLRewriter input body is already used");
    if (response.status === 0) throw new TypeError("HTMLRewriter cannot transform an error response");
    if (response.body === null) return new Response(null, responseInit(response));
    const context = captureAsyncContext();
    const callbacks = new Map<number, { callback: TokenCallback; once: boolean }>();
    const sources = new Map<number, Source>();
    let nextHandler = 0;
    let nextSource = 0;
    const resources: Resources = {
      mutate(operation) {
        if (stopped) throw new TypeError("HTML rewriting has stopped");
        try { return parser.mutate(operation); }
        finally { resources.retire(); }
      },
      retire() {
        for (const { kind, id } of parser.releases()) {
          if (kind === "handler") callbacks.delete(id);
          else {
            const source = sources.get(id);
            if (source && !source.started) { source.reader.releaseLock(); sources.delete(id); }
          }
        }
      },
      addHandler(callback, once = false) {
        const id = nextHandler++;
        callbacks.set(id, { callback, once });
        return id;
      },
      addSource(stream) {
        const id = nextSource++;
        sources.set(id, openSource(stream));
        return id;
      },
    };
    const register = (handlers: Handlers): HandlerSpec => {
      const spec: HandlerSpec = {};
      for (const [name, callback] of handlers) spec[name] = resources.addHandler(callback);
      return spec;
    };
    const handlers: HandlerSpec[] = state.handlers.map(({ selector, handlers }) => ({ selector, ...register(handlers) }));
    for (const document of state.documentHandlers) handlers.push(register(document));
    const parser = htmlRewrite(handlers, response.headers.get("content-type") ?? "");
    const input = openSource(response.body);
    sources.set(nextSource++, input);
    input.started = true;
    input.pending = input.reader.read();
    // The parser consumes this promise below; avoid reporting a rejection
    // before its first native input request arrives.
    input.pending.catch(() => {});
    let stopped = false;
    const stop = async (reason?: unknown, cancelSources = true): Promise<void> => {
      stopped = true;
      parser.cancel();
      callbacks.clear();
      const remaining = [...sources.values()];
      sources.clear();
      // Cleanup must not replace the original parse/handler/cancellation error.
      await Promise.all(remaining.map(async ({ reader, started }) => {
        // Cancelling the output leaves already-consumed input locked.
        if (started && !cancelSources) return;
        try { if (started) await reader.cancel(reason); } catch {}
        finally { reader.releaseLock(); }
      }));
    };
    const readSource = async (id: number): Promise<Uint8Array | undefined> => {
      const source = registered(sources, id);
      source.started = true;
      while (!source.bytes || source.offset === source.bytes.byteLength) {
        const next = await (source.pending ?? source.reader.read());
        source.pending = null;
        if (stopped) return undefined;
        if (next.done) {
          source.reader.releaseLock();
          sources.delete(id);
          return undefined;
        }
        source.bytes = drainedChunkBytes(next.value);
        source.offset = 0;
      }
      const bytes = source.bytes.subarray(source.offset, source.offset + 65536);
      source.offset += bytes.byteLength;
      return bytes;
    };
    const pump = async (controller: ReadableByteStreamController): Promise<void> => {
      try {
        while (!stopped) {
          const event = await parser.next();
          if (stopped) return;
          resources.retire();
          if (event === undefined) {
            await stop();
            controller.close();
            controller.byobRequest?.respond(0);
            return;
          }
          if (event.kind === "input") parser.input(await readSource(event.source));
          else if (event.kind === "token") {
            const handler = registered(callbacks, event.handler);
            if (handler.once) callbacks.delete(event.handler);
            await runInAsyncContext(context, handler.callback, undefined, [event.properties, resources]);
            if (!stopped) parser.done();
          } else {
            controller.enqueue(event.bytes);
            return;
          }
        }
      } catch (error) {
        if (stopped) return;
        await stop(error);
        controller.error(nativeStreamError(error));
      }
    };
    const body = nativeReadableStream({ type: "bytes", start: pump, pull: pump, cancel: () => stop(undefined, false) });
    return new Response(body, responseInit(response));
  }

  get [Symbol.toStringTag](): string { return "HTMLRewriter"; }
}
