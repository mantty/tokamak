import net from "node:net";
import http from "node:http";
import { EventEmitter } from "node:events";
import { Duplex } from "node:stream";
import { connectHandler, handleAsNodeConnection, handleAsNodeRequest, httpServerHandler } from "cloudflare:node";

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

function addressInUse(callback) {
  try { return callback(); }
  catch (error) { return { ...failure(error), syscall: error.syscall ?? null, address: error.address ?? null, port: error.port ?? null }; }
}

// Whether `port` is free in the isolate's port table.
function isFree(port) {
  return attempt(() => { new net.BoundSocket({ port }).close(); return true; });
}

// An inbound socket as the platform hands one to a connect handler: `chunks`
// arrive, then EOF unless `open`; writes and close() calls are recorded.
function inboundSocket(info, chunks = [], open = false) {
  const written = [];
  const closed = Promise.withResolvers();
  let readable;
  let closes = 0;
  const socket = {
    opened: Promise.resolve(info),
    closed: closed.promise,
    readable: new ReadableStream({
      type: "bytes",
      start(controller) {
        readable = controller;
        for (const chunk of chunks) controller.enqueue(new TextEncoder().encode(chunk));
        if (!open) controller.close();
      },
    }),
    writable: new WritableStream({ write(chunk) { written.push(new TextDecoder().decode(chunk)); }, close() { written.push("FIN"); } }),
    close() {
      closes++;
      if (open) {
        readable.close();
        readable.byobRequest?.respond(0);
      }
      closed.resolve();
      return Promise.resolve();
    },
  };
  return { socket, written, closes: () => closes };
}

function boundSocketContracts() {
  const bound = new net.BoundSocket();
  const v6 = new net.BoundSocket({ ipv6Only: true });
  const fixed = new net.BoundSocket({ port: 9100, host: "::1" });
  const shared = new net.BoundSocket({ port: 9101, reusePort: true });
  const result = {
    addresses: [bound.address(), v6.address(), fixed.address(), bound.address() === bound.address(), bound.fd()],
    shape: {
      own: Object.getOwnPropertyNames(bound), prototype: Object.getOwnPropertyNames(net.BoundSocket.prototype),
      symbols: Object.getOwnPropertySymbols(net.BoundSocket.prototype).map(String), statics: Object.getOwnPropertyNames(net.BoundSocket).sort(),
      length: net.BoundSocket.length, call: attempt(() => net.BoundSocket()).error,
    },
    is: [bound, {}, null, bound.address(), net.BoundSocket.prototype].map(value => net.BoundSocket.isBoundSocket(value)),
    conflicts: [
      () => new net.BoundSocket({ port: 9100 }),
      () => new net.BoundSocket({ port: 9101 }),
      () => { const other = new net.BoundSocket({ port: 9101, reusePort: true, host: "127.0.0.1" }); const address = other.address(); other.close(); return address; },
      () => new net.BoundSocket({ port: "9100" }),
    ].map(addressInUse),
    invalid: [
      null, [], "x", 5, { path: "/tmp/socket" }, { path: undefined, port: 9102 }, { host: "localhost" }, { host: 5 }, { host: "" }, { port: -1 },
      { port: "x" }, { port: 65536 }, { port: 1.5 }, { ipv6Only: 1 }, { reusePort: "yes" }, { host: null, ipv6Only: true, port: 9103 },
    ].map(options => attempt(() => { const socket = new net.BoundSocket(options); const address = socket.address(); socket.close(); return address; })),
  };
  fixed.close();
  result.closed = [() => fixed.address(), () => fixed.fd(), () => fixed.close(), () => fixed[Symbol.dispose](), () => isFree(9100)].map(attempt);
  const disposed = new net.BoundSocket({ port: 9104 });
  disposed[Symbol.dispose]();
  result.disposed = [attempt(() => disposed.address()), isFree(9104)];
  bound.close();
  v6.close();
  shared.close();
  result.released = isFree(9101);
  return result;
}

function serverFields(server) {
  return [server.allowHalfOpen, server.pauseOnConnect, server.noDelay, server.keepAlive, server.keepAliveInitialDelay, server.highWaterMark ?? null, server.maxConnections ?? null];
}

// The address a new server reports after listen(...args), or the error listen() throws.
function listenedAddress(args) {
  const server = net.createServer();
  server.on("error", () => {});
  try {
    server.listen(...args);
    return server.address();
  } catch (error) {
    return failure(error);
  } finally {
    server.close();
  }
}

async function serverContracts() {
  const events = [];
  const record = (server, label) => {
    for (const name of ["listening", "close", "error"]) server.on(name, value => events.push([label, name, value instanceof Error ? failure(value) : null]));
  };
  const server = net.createServer();
  record(server, "first");
  const shape = {
    keys: Object.keys(server).filter(name => !name.startsWith("_") && name !== "domain"),
    prototype: Object.getOwnPropertyNames(net.Server.prototype).sort(),
    symbols: Object.getOwnPropertySymbols(net.Server.prototype).map(String),
    emitter: server instanceof EventEmitter,
    lengths: [net.Server.length, net.Server.prototype.listen.length, net.Server.prototype.close.length, net.Server.prototype.getConnections.length],
    call: attempt(() => net.Server()).error,
  };
  const options = [
    undefined, null, {}, { allowHalfOpen: true, pauseOnConnect: 1, noDelay: "yes", keepAlive: true, keepAliveInitialDelay: 2500, highWaterMark: 7 },
    { keepAliveInitialDelay: -5 }, { keepAliveInitialDelay: 1999 }, { keepAliveInitialDelay: "1" }, { allowHalfOpen: 0 }, [], "x", 5,
  ].map(value => attempt(() => serverFields(new net.Server(value))));
  const listeners = [
    () => net.createServer(() => {}).listenerCount("connection"), () => net.createServer({}, "x"), () => net.createServer(null, null),
    () => new net.Server(() => {}, () => {}).listenerCount("connection"), () => net.createServer({}, undefined).listenerCount("connection"),
  ].map(attempt);
  const listened = server.listen(9200) === server;
  const sync = [server.listening, server.address()];
  events.push("sync");
  await turn();
  const again = attempt(() => server.listen(9201));
  const second = net.createServer();
  record(second, "second");
  second.listen(9200);
  const conflicts = [second.listening, second.address(), addressInUse(() => http.createServer().listen(9200)), addressInUse(() => new net.BoundSocket({ port: 9200 }))];
  await turn();
  const variants = [
    [], [0], ["9202"], [9203, "localhost"], [{ port: 9204, host: "::" }], [{ port: 9205, ipv6Only: true }], [{ host: "localhost" }],
    [{ port: 9206, host: "example.test" }], [() => {}], [{ port: 9207, reusePort: true }], [{ port: null }], [9208, "10.0.0.1", () => {}],
    [{ path: "/tmp/socket" }], ["/tmp/socket"], [{ fd: 3 }], [{ handle: {} }], [{ port: -1 }], [{ port: 70000 }], [{ port: "x" }], [{ host: 1 }],
    [{ reusePort: "x" }], [{ port: true }],
  ].map(listenedAddress);
  const bound = new net.BoundSocket({ port: 9210 });
  const adopted = net.createServer().listen(bound);
  const handleBound = new net.BoundSocket({ ipv6Only: true });
  const viaHandle = net.createServer().listen({ handle: handleBound, port: 1 });
  const adoption = [adopted.address(), viaHandle.address(), attempt(() => bound.address()), attempt(() => net.createServer().listen(bound)), addressInUse(() => new net.BoundSocket({ port: 9210 }))];
  adopted.close();
  viaHandle.close();
  adoption.push(isFree(9210));
  server.close(error => events.push(["first", "closed", error ? failure(error) : null]));
  const afterClose = [server.listening, server.address(), isFree(9200)];
  net.createServer().close(error => events.push(["idle", "closed", failure(error)]));
  const counts = [];
  server.getConnections((error, count) => counts.push([error, count]));
  const getConnections = [attempt(() => server.getConnections()), server.ref() === server, server.unref() === server, server.close() === server];
  await turn();
  const disposable = net.createServer().listen(9211);
  await disposable[Symbol.asyncDispose]();
  const relisten = net.createServer();
  record(relisten, "relisten");
  relisten.listen(9212);
  relisten.close();
  relisten.listen(9212);
  const relistened = relisten.address();
  relisten.close();
  await turn();
  return { shape, options, listeners, listened, sync, again, conflicts, variants, adoption, afterClose, counts, getConnections, disposed: [disposable.listening, isFree(9211)], relistened, events };
}

async function connectionContracts() {
  const events = [];
  const server = net.createServer(socket => {
    events.push(["connection", socket.address(), socket._getpeername(), socket.localAddress, socket.localPort, socket.localFamily, socket.remoteAddress, socket.remotePort,
      socket.remoteFamily, socket.server === server, socket._server === server, socket.connecting, socket.pending, socket.readyState, socket.allowHalfOpen, socket.isPaused()]);
    server.getConnections((error, count) => events.push(["connections", error, count]));
    socket.on("data", chunk => {
      events.push(["data", chunk.toString(), socket.bytesRead]);
      socket.write(chunk.toString().toUpperCase());
    });
    for (const name of ["end", "finish", "close"]) socket.on(name, value => events.push([name, value ?? null, socket.bytesWritten]));
  });
  server.listen(9300);
  const counts = [];
  const one = inboundSocket({ remoteAddress: "10.1.2.3:5555", localAddress: "0.0.0.0:9300" }, ["hi", "there"]);
  const handled = connectHandler().connect(one.socket, {}, {});
  server.getConnections((error, count) => counts.push(count));
  await turn();
  server.getConnections((error, count) => counts.push(count));
  events.push(["handled", await handled ?? null, one.written, one.closes()]);
  server.getConnections((error, count) => counts.push(count));

  const halfOpen = net.createServer({ allowHalfOpen: true }, socket => {
    socket.on("data", chunk => events.push(["half-open data", chunk.toString()]));
    socket.on("end", () => {
      events.push(["half-open end", socket.writable]);
      socket.end("late");
    });
    socket.on("close", hadError => events.push(["half-open close", hadError]));
  });
  halfOpen.listen(9301);
  const two = inboundSocket({ remoteAddress: "[::1]:6000", localAddress: "[::]:9301" }, ["x"]);
  await handleAsNodeConnection(two.socket);
  events.push(["half-open written", two.written]);

  const anonymous = net.createServer(socket => {
    events.push(["anonymous", socket.address(), socket._getpeername()]);
    socket.destroy();
  });
  anonymous.listen({ port: 9302, host: "::" });
  await handleAsNodeConnection(inboundSocket({ localAddress: "*:9302" }).socket);
  await handleAsNodeConnection(inboundSocket({ remoteAddress: "peer", localAddress: "[::1]:9302" }).socket);

  const paused = net.createServer({ pauseOnConnect: true }, socket => {
    events.push(["paused", socket.isPaused()]);
    setTimeout(() => {
      events.push(["paused later", socket.bytesRead, socket.readableLength]);
      socket.destroy(new Error("stop"));
    }, 1);
    socket.on("error", error => events.push(["paused error", failure(error)]));
    socket.on("close", hadError => events.push(["paused close", hadError]));
  });
  paused.listen(9303);
  await handleAsNodeConnection(inboundSocket({ localAddress: "0.0.0.0:9303" }, ["held"], true).socket);

  server.maxConnections = 0;
  server.on("drop", data => events.push(["drop", data]));
  const dropped = inboundSocket({ remoteAddress: "[fe80::1]:7", localAddress: "1.2.3.4:9300" });
  events.push(["dropped", await asyncAttempt(() => handleAsNodeConnection(dropped.socket)) ?? null, dropped.closes()]);

  const lingering = net.createServer(socket => {
    events.push("lingering connection");
    setTimeout(() => {
      events.push("lingering destroy");
      socket.destroy();
    }, 1);
  });
  lingering.on("close", () => events.push("lingering closed"));
  lingering.listen(9304);
  const pending = handleAsNodeConnection(inboundSocket({ localAddress: "0.0.0.0:9304" }, [], true).socket);
  await Promise.resolve();
  await Promise.resolve();
  lingering.close(() => events.push("lingering close callback"));
  events.push("lingering close called");
  await pending;
  await turn();

  const closedFirst = net.createServer(() => events.push("never"));
  closedFirst.listen(9305);
  const late = inboundSocket({ localAddress: "0.0.0.0:9305" });
  const routed = handleAsNodeConnection(late.socket);
  closedFirst.close();
  events.push(["closed first", await asyncAttempt(() => routed) ?? null, late.closes()]);

  const web = http.createServer(() => {}).listen(9306);
  const errors = await Promise.all([
    { localAddress: "0.0.0.0:9399" }, {}, { localAddress: "x:70000" }, { localAddress: 9300 }, { localAddress: "127.0.0.1:9306" }, { localAddress: "[::1]" },
  ].map(info => asyncAttempt(() => handleAsNodeConnection({ opened: Promise.resolve(info) }))));
  errors.push(await asyncAttempt(() => handleAsNodeConnection({ opened: Promise.reject(new RangeError("refused")) })));
  errors.push(await asyncAttempt(() => handleAsNodeRequest(9300, new Request("http://example.test/"))));
  errors.push((await asyncAttempt(() => handleAsNodeConnection(null))).error);
  web.close();
  for (const listening of [server, halfOpen, anonymous, paused]) listening.close();
  const handler = connectHandler();
  return {
    handler: { own: Object.getOwnPropertyNames(handler), type: typeof handler.connect, length: handler.connect.length, fresh: handler !== connectHandler() },
    lengths: [connectHandler.length, handleAsNodeConnection.length],
    counts, events, errors,
  };
}

async function socketContracts() {
  const constructed = [
    () => net.Socket() instanceof net.Socket, () => new net.Socket({ objectMode: true }), () => new net.Socket({ readableObjectMode: true }),
    () => new net.Socket({ writableObjectMode: 1 }), () => new net.Socket(3), () => new net.Socket({ fd: 3 }), () => new net.Socket({ keepAliveInitialDelay: "1" }),
    () => new net.Socket({ signal: {} }), () => new net.Socket({ handle: 5 }), () => new net.Socket({ allowHalfOpen: 1 }).allowHalfOpen,
  ].map(attempt);
  const fresh = new net.Socket();
  const state = [
    fresh.connecting, fresh.pending, fresh.readyState, fresh.bufferSize ?? null, fresh.bytesRead, fresh.bytesWritten, fresh._bytesDispatched, fresh.server, fresh._server,
    fresh._host, fresh._handle, fresh.timeout ?? null, fresh.autoSelectFamilyAttemptedAddresses, fresh.ref() ?? null, fresh.unref() ?? null,
    fresh.setNoDelay(true) === fresh, fresh.setKeepAlive(true, 1) === fresh, fresh.address(), fresh.remoteAddress ?? null, fresh.localPort ?? null,
  ];
  const connects = [
    [], [{}], [{ port: true }], [{ port: -1 }], [{ port: "x" }], [{ port: null }], [{ port: 1, localAddress: "nope" }], [{ port: 1, localPort: "1" }],
    [{ port: 1, autoSelectFamily: 1 }], [{ port: 1, autoSelectFamilyAttemptTimeout: 0 }], [{ port: 1, autoSelectFamilyAttemptTimeout: 1.5 }],
    [{ port: 1, lookup: 5 }], [{ path: "/tmp/socket" }], ["/tmp/socket"],
  ].map(args => attempt(() => {
    const socket = new net.Socket();
    socket.on("error", () => {});
    socket.connect(...args);
    socket.destroy();
    return "connected";
  }));
  const connecting = new net.Socket();
  connecting.on("error", () => {});
  const attempts = [];
  connecting.on("connectionAttempt", (...args) => attempts.push(args));
  connecting.connect({ host: "::1", port: 1, localAddress: "::1" });
  const described = [
    connecting.address(), connecting._getpeername(), connecting._host, connecting.autoSelectFamilyAttemptedAddresses, attempts, connecting.connecting,
    connecting.pending, connecting.readyState, attempt(() => connecting.connect({ port: 1 })),
  ];
  connecting.destroy();
  described.push(connecting.address(), connecting.connecting);

  const reserved = net.connect({ host: "127.0.0.1", port: 1, localPort: 9400 });
  reserved.on("error", () => {});
  const reservation = [reserved.address(), addressInUse(() => new net.BoundSocket({ port: 9400 })), reserved.connecting];
  reserved.destroy();
  reservation.push(isFree(9400));

  const holder = new net.BoundSocket({ port: 9401 });
  const conflicted = new net.Socket();
  const socketEvents = [];
  conflicted.on("error", error => socketEvents.push(["conflict error", failure(error)]));
  conflicted.on("close", hadError => socketEvents.push(["conflict close", hadError]));
  conflicted.connect({ host: "127.0.0.1", port: 1, localPort: 9401 });
  holder.close();

  const handle = new net.BoundSocket({ port: 9402, host: "127.0.0.1" });
  const adopting = new net.Socket({ handle });
  adopting.on("error", () => {});
  const adopted = [adopting.address(), attempt(() => handle.address()), attempt(() => new net.Socket({ handle })), attempt(() => adopting.connect({ port: 1, localPort: 5 }))];
  const v6Handle = new net.BoundSocket({ ipv6Only: true });
  const wildcard = new net.Socket({ handle: v6Handle });
  wildcard.on("error", () => {});
  wildcard.connect({ host: "::1", port: 1 });
  adopted.push(wildcard.address());
  wildcard.destroy();
  const reconnected = new net.Socket();
  reconnected.on("error", () => {});
  reconnected.connect(1, "127.0.0.1");
  adopted.push(reconnected.address());
  reconnected.destroy();
  adopting.destroy();
  adopted.push(isFree(9402));

  const lookups = [];
  const viaLookup = new net.Socket();
  viaLookup.on("error", () => {});
  viaLookup.on("lookup", (error, address, family, host) => lookups.push(["lookup", error ?? null, address, family, host]));
  viaLookup.connect({ host: "example.test", port: 1, family: 6, hints: 32, lookup: (host, options, callback) => {
    lookups.push(["called", host, options]);
    callback(null, "127.0.0.1", 4);
  } });
  lookups.push(viaLookup._getpeername(), viaLookup.address(), viaLookup._host);
  viaLookup.destroy();
  const failedLookup = new net.Socket();
  failedLookup.on("error", error => socketEvents.push(["lookup error", failure(error)]));
  failedLookup.on("close", hadError => socketEvents.push(["lookup close", hadError]));
  failedLookup.connect({ host: "example.test", port: 1, lookup: (host, options, callback) => callback(Object.assign(new Error("not found"), { code: "ENOTFOUND" })) });
  const badLookup = [
    attempt(() => new net.Socket().connect({ host: "example.test", port: 1, lookup: (host, options, callback) => callback(null, "nope", 4) })),
    attempt(() => new net.Socket().connect({ host: "example.test", port: 1, lookup: (host, options, callback) => callback(null, "127.0.0.1", 5) })),
  ];

  const timed = new net.Socket();
  const timeouts = [attempt(() => timed.setTimeout(-1)), attempt(() => timed.setTimeout(Infinity)), attempt(() => timed.setTimeout("5")), timed.timeout, attempt(() => timed.setTimeout(0, 1)), timed.setTimeout(0) === timed];
  timeouts.push(await new Promise(resolve => {
    const socket = new net.Socket();
    socket.setTimeout(1, () => resolve(["timeout", socket.timeout]));
  }));
  const destroyed = new net.Socket();
  destroyed.destroy();
  timeouts.push(destroyed.setTimeout(-1) === destroyed);

  const unconnected = new net.Socket();
  unconnected.on("error", error => socketEvents.push(["write error", failure(error)]));
  unconnected.on("close", hadError => socketEvents.push(["write close", hadError]));
  unconnected.write("x", error => socketEvents.push(["write callback", failure(error)]));
  const reset = new net.Socket();
  reset.on("error", error => socketEvents.push(["reset error", failure(error)]));
  socketEvents.push(["reset returns", reset.resetAndDestroy() === reset]);
  const failing = new net.Socket();
  failing.on("error", error => socketEvents.push(["destroy error", failure(error)]));
  failing.on("close", hadError => socketEvents.push(["destroy close", hadError]));
  failing.destroy(new Error("broken"));
  const plain = new net.Socket();
  plain.on("close", hadError => socketEvents.push(["plain close", hadError]));
  socketEvents.push(["destroySoon", plain.destroySoon() ?? null]);
  const aborted = new net.Socket({ signal: AbortSignal.abort() });
  aborted.on("error", error => socketEvents.push(["abort error", failure(error)]));
  aborted.on("close", hadError => socketEvents.push(["abort close", hadError]));
  await turn();
  const abort = [aborted._aborted, attempt(() => aborted.connect({ port: 1 })), await new Promise(resolve => aborted.connect({ port: 1 }, error => resolve(failure(error))))];
  await turn();
  return { constructed, state, connects, described, reservation, adopted, lookups, badLookup, timeouts, abort, events: socketEvents };
}

function moduleContracts() {
  const autoSelect = [
    net.getDefaultAutoSelectFamily(), net.getDefaultAutoSelectFamilyAttemptTimeout(), attempt(() => net.setDefaultAutoSelectFamily(false)),
    attempt(() => net.setDefaultAutoSelectFamily(true)), attempt(() => net.setDefaultAutoSelectFamilyAttemptTimeout(5)), net.getDefaultAutoSelectFamilyAttemptTimeout(),
    attempt(() => net.setDefaultAutoSelectFamilyAttemptTimeout(250)), net.getDefaultAutoSelectFamilyAttemptTimeout(), attempt(() => net.setDefaultAutoSelectFamilyAttemptTimeout(0)),
    attempt(() => net.setDefaultAutoSelectFamilyAttemptTimeout(1.5)), attempt(() => net.setDefaultAutoSelectFamilyAttemptTimeout("5")),
  ];
  net.setDefaultAutoSelectFamilyAttemptTimeout(10);
  return {
    stream: [net.Stream === net.Socket, new net.Socket() instanceof Duplex, Object.getPrototypeOf(net.Socket) === Duplex],
    autoSelect,
    normalized: [[], [80], ["80"], ["/tmp/x"], [80, "host"], [80, "host", () => {}], [{ port: 1 }], [null], [() => {}], ["-1"]].map(args => {
      const normalized = net._normalizeArgs(args);
      return [normalized.length, normalized[0], typeof normalized[1], Object.getOwnPropertySymbols(normalized).length];
    }),
    lengths: [net.connect.length, net.createConnection.length, net.createServer.length, net.Server.length, net.Socket.length, net.BoundSocket.length, net._normalizeArgs.length],
  };
}

async function httpPortContracts() {
  const order = [];
  const web = http.createServer(() => {});
  web.listen(0, () => order.push("listening callback"));
  order.push("sync");
  const port = web.address().port;
  const bound = new net.BoundSocket();
  const contention = await new Promise(resolve => {
    const server = net.createServer();
    server.on("error", error => resolve([server.listening, failure(error)]));
    server.listen(port);
  });
  const results = {
    ephemeral: typeof port === "number" && port >= 49152,
    contention,
    boundConflict: addressInUse(() => http.createServer().listen(bound.address().port)),
    twice: attempt(() => web.listen(0)),
    listens: [[], [{}], [{ port: "9501" }], ["/tmp/socket"], ["x"], [{ port: true }], [9502, "0.0.0.0"], [-1], [70000]].map(args => attempt(() => {
      const server = http.createServer();
      server.listen(...args);
      const { port: listened, ...address } = server.address();
      server.close();
      return { ...address, port: listened >= 49152 ? "ephemeral" : listened };
    })),
  };
  bound.close();
  web.close(error => order.push(["close callback", error ?? null]));
  results.afterClose = [web.listening, web.address(), isFree(port)];
  const unlistened = http.createServer((request, response) => {
    const { env, ctx, cf, ...rest } = request.cloudflare;
    response.end(JSON.stringify([Object.keys(request.cloudflare), env ?? null, ctx ?? null, cf ?? null, rest]));
  });
  const handler = httpServerHandler(unlistened);
  const handled = await handler.fetch(new Request("http://example.test/"));
  const bindings = await handleAsNodeRequest(unlistened.address().port, new Request("http://example.test/"), { name: "env" }, { name: "ctx" });
  results.handler = [unlistened.listening, unlistened.address().port >= 49152, handled.status, await handled.text(), await bindings.text()];
  unlistened.close();
  await turn();
  results.order = order;
  return results;
}

export async function netContracts() {
  return {
    boundSocket: boundSocketContracts(),
    server: await serverContracts(),
    connections: await connectionContracts(),
    socket: await socketContracts(),
    module: moduleContracts(),
    httpPorts: await httpPortContracts(),
  };
}
