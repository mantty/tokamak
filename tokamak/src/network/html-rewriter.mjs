import { htmlRewrite, htmlValidateSelector } from "tokamak:host";
import { ReadableStream, nativeReadableStream, nativeStreamError } from "../streams/web.mjs";
import { captureAsyncContext, runInAsyncContext } from "../builtins/async-context.mjs";

const elementStates = new WeakMap();
const textStates = new WeakMap();
const commentStates = new WeakMap();
const endTagStates = new WeakMap();
const documentEndStates = new WeakMap();
const attributesStates = new WeakMap();
const rewriterStates = new WeakMap();
const doctypeStates = new WeakMap();
const expiredTokens = new WeakSet();

function contentType(options) {
  return options != null && options.html === true ? "html" : "text";
}

function contentOperation(state, name, value, options) {
  const kind = contentType(options);
  if (value instanceof ReadableStream || value instanceof Response) {
    if (!state.resources) throw new TypeError("This HTML token requires string content");
    const stream = value instanceof Response ? value.body : value;
    value = stream === null ? "" : { source: state.resources.addSource(stream) };
  } else {
    if (typeof value === "symbol") throw new TypeError("Cannot convert a Symbol to a string");
    value = String(value);
  }
  applyMutation(state, { name, value, contentType: kind });
}

function requireState(states, value) {
  const state = states.get(value);
  if (state === undefined) throw new TypeError("Illegal invocation");
  if (expiredTokens.has(value)) throw new TypeError("This content token is no longer valid. Content tokens are only valid during the execution of the relevant content handler.");
  return state;
}

function applyMutation(state, operation) {
  Object.assign(state, state.mutate(operation));
  if (operation.name === "setAttribute" || operation.name === "removeAttribute") state.attributeVersion++;
}

function asciiLower(value) { return String(value).replace(/[A-Z]/g, letter => letter.toLowerCase()); }

function initialAttributes(properties) {
  return Array.from(properties.attributes ?? [], attribute => ({
    name: asciiLower(Array.isArray(attribute) ? attribute[0] : attribute.name),
    value: String(Array.isArray(attribute) ? attribute[1] : attribute.value),
  }));
}

class AttributesIterator {
  constructor(owner, entries, version) {
    attributesStates.set(this, { owner, entries, index: 0, version });
  }

  next() {
    const state = requireState(attributesStates, this);
    const element = requireState(elementStates, state.owner);
    if (state.version !== element.attributeVersion) throw new Error("The attributes have been modified during iteration");
    if (state.index === state.entries.length) return { value: undefined, done: true };
    return { value: state.entries[state.index++], done: false };
  }

  [Symbol.iterator]() { return this; }

  get [Symbol.toStringTag]() { return "AttributesIterator"; }
}

class Element {
  constructor(properties, mutate, resources) {
    const state = {
      attributes: initialAttributes(properties),
      attributeVersion: 0,
      namespaceURI: properties.namespaceURI,
      mutate,
      resources,
      removed: Boolean(properties.removed),
      tagName: String(properties.tagName),
    };
    elementStates.set(this, state);
    Object.defineProperties(this, {
      tagName: {
        enumerable: true,
        get() { return requireState(elementStates, this).tagName; },
        set(value) {
          const state = requireState(elementStates, this);
          applyMutation(state, { name: "setTagName", value: String(value) });
        },
      },
      namespaceURI: { enumerable: true, get() { return requireState(elementStates, this).namespaceURI; } },
      attributes: {
        enumerable: true,
        get() {
          const state = requireState(elementStates, this);
          return new AttributesIterator(this, state.attributes.map(({ name, value }) => [name, value]), state.attributeVersion);
        },
      },
      removed: { enumerable: true, get() { return requireState(elementStates, this).removed; } },
    });
  }

  getAttribute(name) {
    const state = requireState(elementStates, this);
    const key = asciiLower(name);
    return state.attributes.find(attribute => attribute.name === key)?.value ?? null;
  }

  hasAttribute(name) {
    const state = requireState(elementStates, this);
    const key = asciiLower(name);
    return state.attributes.some(attribute => attribute.name === key);
  }

  setAttribute(name, value) {
    const state = requireState(elementStates, this);
    const attribute = asciiLower(name);
    const attributeValue = String(value);
    applyMutation(state, { name: "setAttribute", attribute, value: attributeValue });
    return this;
  }

  removeAttribute(name) {
    const state = requireState(elementStates, this);
    const attribute = asciiLower(name);
    applyMutation(state, { name: "removeAttribute", attribute });
    return this;
  }

  before(value, options) {
    contentOperation(requireState(elementStates, this), "before", value, options);
    return this;
  }

  after(value, options) {
    contentOperation(requireState(elementStates, this), "after", value, options);
    return this;
  }

  prepend(value, options) {
    contentOperation(requireState(elementStates, this), "prepend", value, options);
    return this;
  }

  append(value, options) {
    contentOperation(requireState(elementStates, this), "append", value, options);
    return this;
  }

  replace(value, options) {
    const state = requireState(elementStates, this);
    contentOperation(state, "replace", value, options);
    return this;
  }

  setInnerContent(value, options) {
    contentOperation(requireState(elementStates, this), "setInnerContent", value, options);
    return this;
  }

  remove() {
    const state = requireState(elementStates, this);
    applyMutation(state, { name: "remove" });
    return this;
  }

  removeAndKeepContent() {
    const state = requireState(elementStates, this);
    applyMutation(state, { name: "removeAndKeepContent" });
    return this;
  }

  onEndTag(handler) {
    if (typeof handler !== "function") throw new TypeError("HTMLRewriter end-tag handler must be a function");
    const state = requireState(elementStates, this);
    applyMutation(state, {
      name: "onEndTag",
      handler: state.resources.addHandler(async properties => {
        const token = new EndTag(properties, state.resources.mutate, state.resources);
        try { await handler(token); }
        finally { expiredTokens.add(token); }
      }, true),
    });
  }

  get [Symbol.toStringTag]() { return "Element"; }
}

class Text {
  constructor(properties, mutate, resources) {
    const state = {
      lastInTextNode: Boolean(properties.lastInTextNode),
      mutate,
      resources,
      removed: Boolean(properties.removed),
      text: String(properties.text),
    };
    textStates.set(this, state);
    Object.defineProperties(this, {
      text: {
        enumerable: true,
        get() { return requireState(textStates, this).text; },
      },
      lastInTextNode: { enumerable: true, get() { return requireState(textStates, this).lastInTextNode; } },
      removed: { enumerable: true, get() { return requireState(textStates, this).removed; } },
    });
  }

  before(value, options) {
    contentOperation(requireState(textStates, this), "before", value, options);
    return this;
  }

  after(value, options) {
    contentOperation(requireState(textStates, this), "after", value, options);
    return this;
  }

  replace(value, options) {
    const state = requireState(textStates, this);
    contentOperation(state, "replace", value, options);
    return this;
  }

  remove() {
    const state = requireState(textStates, this);
    applyMutation(state, { name: "remove" });
    return this;
  }

  get [Symbol.toStringTag]() { return "Text"; }
}

class Comment {
  constructor(properties, mutate) {
    const state = { mutate, removed: Boolean(properties.removed), text: String(properties.text) };
    commentStates.set(this, state);
    Object.defineProperties(this, {
      text: {
        enumerable: true,
        get() { return requireState(commentStates, this).text; },
        set(value) {
          const state = requireState(commentStates, this);
          applyMutation(state, { name: "setText", value: String(value) });
        },
      },
      removed: { enumerable: true, get() { return requireState(commentStates, this).removed; } },
    });
  }

  before(value, options) {
    contentOperation(requireState(commentStates, this), "before", value, options);
    return this;
  }

  after(value, options) {
    contentOperation(requireState(commentStates, this), "after", value, options);
    return this;
  }

  replace(value, options) {
    const state = requireState(commentStates, this);
    contentOperation(state, "replace", value, options);
    return this;
  }

  remove() {
    const state = requireState(commentStates, this);
    applyMutation(state, { name: "remove" });
    return this;
  }

  get [Symbol.toStringTag]() { return "Comment"; }
}

class EndTag {
  constructor(properties, mutate, resources) {
    endTagStates.set(this, { mutate, resources, name: String(properties.name), removed: Boolean(properties.removed) });
    Object.defineProperties(this, {
      name: { enumerable: true, get() { return requireState(endTagStates, this).name; } },
    });
  }

  before(value, options) {
    contentOperation(requireState(endTagStates, this), "before", value, options);
  }

  after(value, options) {
    contentOperation(requireState(endTagStates, this), "after", value, options);
  }

  remove() {
    const state = requireState(endTagStates, this);
    applyMutation(state, { name: "remove" });
  }

  get [Symbol.toStringTag]() { return "EndTag"; }
}

class Doctype {
  constructor(properties) {
    doctypeStates.set(this, properties);
    Object.defineProperties(this, {
      name: { enumerable: true, get() { return requireState(doctypeStates, this).name ?? null; } },
      publicId: { enumerable: true, get() { return requireState(doctypeStates, this).publicId ?? null; } },
      systemId: { enumerable: true, get() { return requireState(doctypeStates, this).systemId ?? null; } },
    });
  }

  get [Symbol.toStringTag]() { return "Doctype"; }
}

class DocumentEnd {
  constructor(_properties, mutate) {
    documentEndStates.set(this, { mutate });
  }

  append(value, options) {
    contentOperation(requireState(documentEndStates, this), "append", value, options);
    return this;
  }

  get [Symbol.toStringTag]() { return "DocumentEnd"; }
}

function callbackHandler(registered, name, Token) {
  const callback = registered?.[name];
  if (callback === undefined) return undefined;
  if (typeof callback !== "function") throw new TypeError(`HTMLRewriter ${name} handler must be a function`);
  return async (properties, resources) => {
    const token = new Token(properties, resources.mutate, resources);
    try { await callback.call(registered, token); }
    finally { expiredTokens.add(token); }
  };
}

function captureHandlers(registered, definitions) {
  const handlers = {};
  for (const [name, Token] of definitions) {
    const handler = callbackHandler(registered, name, Token);
    if (handler !== undefined) handlers[name] = handler;
  }
  return handlers;
}

function responseInit(response) {
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

  on(selector, handlers) {
    if (arguments.length < 2) {
      throw new TypeError("Invalid HTMLRewriter handler");
    }
    selector = `${selector}`;
    htmlValidateSelector(selector);
    requireState(rewriterStates, this).handlers.push({
      selector,
      handlers: captureHandlers(handlers, [["element", Element], ["text", Text], ["comments", Comment]]),
    });
    return this;
  }

  onDocument(handlers) {
    if (arguments.length < 1) throw new TypeError("Invalid HTMLRewriter document handler");
    requireState(rewriterStates, this).documentHandlers.push(
      captureHandlers(handlers ?? {}, [["doctype", Doctype], ["comments", Comment], ["text", Text], ["end", DocumentEnd]]),
    );
    return this;
  }

  transform(response) {
    const state = requireState(rewriterStates, this);
    if (!(response instanceof Response)) throw new TypeError("HTMLRewriter.transform requires a Response");
    if (response.bodyUsed || response.body?.locked) throw new TypeError("HTMLRewriter input body is already used");
    if (response.status === 0) throw new TypeError("HTMLRewriter cannot transform an error response");
    if (response.body === null) return new Response(null, responseInit(response));
    const context = captureAsyncContext();
    const callbacks = new Map();
    const sources = new Map();
    let nextHandler = 0;
    let nextSource = 0;
    const resources = {
      mutate(operation) {
        if (stopped) throw new TypeError("HTML rewriting has stopped");
        try { return parser.mutate(JSON.stringify(operation)); }
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
        sources.set(id, { reader: stream.getReader(), bytes: null, offset: 0, started: false });
        return id;
      },
    };
    const register = handlers => Object.fromEntries(Object.entries(handlers).map(([name, callback]) => [name, resources.addHandler(callback)]));
    const handlers = state.handlers.map(({ selector, handlers }) => ({ selector: String(selector), ...register(handlers) }));
    for (const document of state.documentHandlers) handlers.push(register(document));
    const parser = htmlRewrite(JSON.stringify(handlers), response.headers.get("content-type") ?? "");
    resources.addSource(response.body);
    const input = sources.get(0);
    input.started = true;
    input.pending = input.reader.read();
    // The parser consumes this promise below; avoid reporting a rejection
    // before its first native input request arrives.
    input.pending.catch(() => {});
    let stopped = false;
    const stop = async (reason, cancelSources = true) => {
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
    const readSource = async id => {
      const source = sources.get(id);
      source.started = true;
      while (!source.bytes || source.offset === source.bytes.byteLength) {
        const next = await (source.pending ?? source.reader.read());
        source.pending = null;
        if (stopped) return null;
        if (next.done) {
          source.reader.releaseLock();
          sources.delete(id);
          return null;
        }
        if (next.value instanceof ArrayBuffer) source.bytes = new Uint8Array(next.value);
        else if (ArrayBuffer.isView(next.value)) source.bytes = new Uint8Array(next.value.buffer, next.value.byteOffset, next.value.byteLength);
        else throw new TypeError("HTMLRewriter input streams must contain bytes");
        source.offset = 0;
      }
      const bytes = source.bytes.subarray(source.offset, source.offset + 65536);
      source.offset += bytes.byteLength;
      return bytes;
    };
    const pump = async controller => {
        try {
          while (!stopped) {
            const event = await parser.next();
            if (stopped) return;
            resources.retire();
            if (event === null) {
              await stop();
              controller.close();
              controller.byobRequest?.respond(0);
              return;
            }
            if (event.kind === "input") parser.reply(await readSource(event.source));
            else if (event.kind === "token") {
              const handler = callbacks.get(event.handler);
              if (handler.once) callbacks.delete(event.handler);
              await runInAsyncContext(context, handler.callback, undefined, [event.properties, resources]);
              if (!stopped) parser.reply(undefined);
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

  get [Symbol.toStringTag]() { return "HTMLRewriter"; }
}
