import { htmlRewrite } from "tokamak:host";
import { ReadableStream } from "../streams/web.mjs";

const elementStates = new WeakMap();
const textStates = new WeakMap();
const commentStates = new WeakMap();
const endTagStates = new WeakMap();
const documentEndStates = new WeakMap();
const attributesStates = new WeakMap();
const rewriterStates = new WeakMap();

function contentType(options) {
  return options != null && options.html === true ? "html" : "text";
}

function contentOperation(state, name, value, options) {
  state.operations.push({ name, value: String(value), contentType: contentType(options) });
}

function requireState(states, value) {
  const state = states.get(value);
  if (state === undefined) throw new TypeError("Illegal invocation");
  return state;
}

function initialAttributes(properties) {
  return Array.from(properties.attributes ?? [], attribute => ({
    name: String(Array.isArray(attribute) ? attribute[0] : attribute.name).toLowerCase(),
    value: String(Array.isArray(attribute) ? attribute[1] : attribute.value),
  }));
}

class AttributesIterator {
  constructor(entries) {
    attributesStates.set(this, { entries, index: 0 });
  }

  next() {
    const state = requireState(attributesStates, this);
    if (state.index === state.entries.length) return { value: undefined, done: true };
    return { value: state.entries[state.index++], done: false };
  }

  [Symbol.iterator]() { return this; }

  get [Symbol.toStringTag]() { return "AttributesIterator"; }
}

class Element {
  constructor(properties, operations) {
    const state = {
      attributes: initialAttributes(properties),
      namespaceURI: properties.namespaceURI,
      operations,
      removed: false,
      tagName: String(properties.tagName),
    };
    elementStates.set(this, state);
    Object.defineProperties(this, {
      tagName: {
        enumerable: true,
        get() { return requireState(elementStates, this).tagName; },
        set(value) {
          const state = requireState(elementStates, this);
          state.tagName = String(value);
          state.operations.push({ name: "setTagName", value: state.tagName });
        },
      },
      namespaceURI: { enumerable: true, value: state.namespaceURI },
      attributes: {
        enumerable: true,
        get() {
          const state = requireState(elementStates, this);
          return new AttributesIterator(state.attributes.map(({ name, value }) => [name, value]));
        },
      },
      removed: { enumerable: true, get() { return requireState(elementStates, this).removed; } },
    });
  }

  getAttribute(name) {
    const state = requireState(elementStates, this);
    const key = String(name).toLowerCase();
    return state.attributes.find(attribute => attribute.name === key)?.value ?? null;
  }

  hasAttribute(name) {
    const state = requireState(elementStates, this);
    const key = String(name).toLowerCase();
    return state.attributes.some(attribute => attribute.name === key);
  }

  setAttribute(name, value) {
    const state = requireState(elementStates, this);
    const attribute = String(name).toLowerCase();
    const attributeValue = String(value);
    const existing = state.attributes.find(item => item.name === attribute);
    if (existing === undefined) state.attributes.push({ name: attribute, value: attributeValue });
    else existing.value = attributeValue;
    state.operations.push({ name: "setAttribute", attribute, value: attributeValue });
    return this;
  }

  removeAttribute(name) {
    const state = requireState(elementStates, this);
    const attribute = String(name).toLowerCase();
    state.attributes = state.attributes.filter(item => item.name !== attribute);
    state.operations.push({ name: "removeAttribute", attribute });
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
    state.removed = true;
    contentOperation(state, "replace", value, options);
    return this;
  }

  setInnerContent(value, options) {
    contentOperation(requireState(elementStates, this), "setInnerContent", value, options);
    return this;
  }

  remove() {
    const state = requireState(elementStates, this);
    state.removed = true;
    state.operations.push({ name: "remove" });
    return this;
  }

  removeAndKeepContent() {
    const state = requireState(elementStates, this);
    state.removed = true;
    state.operations.push({ name: "removeAndKeepContent" });
    return this;
  }

  onEndTag(handler) {
    if (typeof handler !== "function") throw new TypeError("HTMLRewriter end-tag handler must be a function");
    const state = requireState(elementStates, this);
    state.operations.push({
      name: "onEndTag",
      handler: properties => {
        const operations = [];
        handler(new EndTag(properties, operations));
        return operations;
      },
    });
  }

  get [Symbol.toStringTag]() { return "Element"; }
}

class Text {
  constructor(properties, operations) {
    const state = {
      lastInTextNode: Boolean(properties.lastInTextNode),
      operations,
      removed: false,
      text: String(properties.text),
    };
    textStates.set(this, state);
    Object.defineProperties(this, {
      text: {
        enumerable: true,
        get() { return requireState(textStates, this).text; },
      },
      lastInTextNode: { enumerable: true, value: state.lastInTextNode },
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
    state.removed = true;
    contentOperation(state, "replace", value, options);
    return this;
  }

  remove() {
    const state = requireState(textStates, this);
    state.removed = true;
    state.operations.push({ name: "remove" });
    return this;
  }

  get [Symbol.toStringTag]() { return "Text"; }
}

class Comment {
  constructor(properties, operations) {
    const state = { operations, removed: false, text: String(properties.text) };
    commentStates.set(this, state);
    Object.defineProperties(this, {
      text: {
        enumerable: true,
        get() { return requireState(commentStates, this).text; },
        set(value) {
          const state = requireState(commentStates, this);
          state.text = String(value);
          state.operations.push({ name: "setText", value: state.text });
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
    state.removed = true;
    contentOperation(state, "replace", value, options);
    return this;
  }

  remove() {
    const state = requireState(commentStates, this);
    state.removed = true;
    state.operations.push({ name: "remove" });
    return this;
  }

  get [Symbol.toStringTag]() { return "Comment"; }
}

class EndTag {
  constructor(properties, operations) {
    endTagStates.set(this, { operations, removed: Boolean(properties.removed) });
    Object.defineProperties(this, {
      name: { enumerable: true, value: String(properties.name) },
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
    state.removed = true;
    state.operations.push({ name: "remove" });
  }

  get [Symbol.toStringTag]() { return "EndTag"; }
}

class Doctype {
  constructor(properties) {
    Object.defineProperties(this, {
      name: { enumerable: true, value: properties.name ?? null },
      publicId: { enumerable: true, value: properties.publicId ?? null },
      systemId: { enumerable: true, value: properties.systemId ?? null },
    });
  }

  get [Symbol.toStringTag]() { return "Doctype"; }
}

class DocumentEnd {
  constructor(_properties, operations) {
    documentEndStates.set(this, { operations });
  }

  append(value, options) {
    contentOperation(requireState(documentEndStates, this), "append", value, options);
    return this;
  }

  get [Symbol.toStringTag]() { return "DocumentEnd"; }
}

function callbackHandler(registered, name, Token) {
  const callback = registered?.[name];
  if (typeof callback !== "function") return undefined;
  return properties => {
    const operations = [];
    callback.call(registered, new Token(properties, operations));
    return operations;
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
    const { handlers: registeredHandlers, documentHandlers } = state;
    const handlers = registeredHandlers.map(({ selector, handlers: callbacks }) => ({
      selector: String(selector),
      ...callbacks,
    }));
    for (const callbacks of documentHandlers) handlers.push({ selector: "*", document: callbacks });
    const body = new ReadableStream({
      async start(controller) {
        try {
          const rewritten = htmlRewrite(await response.text(), handlers);
          if (rewritten.length > 0) controller.enqueue(new TextEncoder().encode(rewritten));
          controller.close();
        } catch (error) {
          controller.error(error);
        }
      },
    });
    return new Response(body, responseInit(response));
  }

  get [Symbol.toStringTag]() { return "HTMLRewriter"; }
}
