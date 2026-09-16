class Span {
  constructor() {
    throw new TypeError("Illegal constructor");
  }

  get isTraced() { return false; }
  setAttribute() { return this; }
  setAttributes() { return this; }
  end() {}
}

class Tracing {
  startSpan(name) {
    if (typeof name !== "string") throw new TypeError("parameter 1 is not of type 'string'");
    return Object.create(Span.prototype);
  }

  startActiveSpan(name, callback) {
    if (typeof callback !== "function") throw new TypeError("parameter 2 is not of type 'Function'");
    this.startSpan(name);
    return callback();
  }

  enterSpan(name, callback) {
    if (typeof callback !== "function") throw new TypeError("parameter 2 is not of type 'Function'");
    this.startSpan(name);
    return callback();
  }
}

Tracing.prototype.Span = Span;

export function createTracing() {
  return new Tracing();
}
