import EventEmitter from "../events/events.mjs";
import { installConsoleGlobal } from "../globals/console.mjs";
import process, { installProcessGlobals } from "../globals/process.mjs";
import { installWebGlobals } from "../globals/web.mjs";
import { installWebSocketGlobals } from "../network/websocket.mjs";
import { createTracing } from "./tracing.mjs";
import streams from "../streams/node.mjs";
import webStreams from "../streams/web.mjs";
import streamConsumers from "../node/stream-consumers.mjs";
import streamPromises from "../node/stream-promises.mjs";
import fs from "node:fs";
import fsPromises from "node:fs/promises";
import assert from "../node/assert.mjs";
import asyncHooks from "../node/async-hooks.mjs";
import buffer from "../node/buffer.mjs";
import childProcess from "../node/child-process.mjs";
import cluster from "../node/cluster.mjs";
import nodeConsole from "../node/console.mjs";
import constants from "../node/constants.mjs";
import nodeCrypto from "../node/crypto.mjs";
import diagnosticsChannel from "../node/diagnostics-channel.mjs";
import dns from "../node/dns.mjs";
import dnsPromises from "../node/dns-promises.mjs";
import dgram from "../node/dgram.mjs";
import domain from "../node/domain.mjs";
import http from "../node/http.mjs";
import http2 from "../node/http2.mjs";
import inspector from "../node/inspector.mjs";
import internalHttpAgent from "../node/internal-http-agent.mjs";
import internalHttpClient from "../node/internal-http-client.mjs";
import internalHttpCommon from "../node/internal-http-common.mjs";
import internalHttpIncoming from "../node/internal-http-incoming.mjs";
import internalHttpOutgoing from "../node/internal-http-outgoing.mjs";
import internalHttpServer from "../node/internal-http-server.mjs";
import https from "../node/https.mjs";
import nodeModule from "../node/module.mjs";
import net from "../node/net.mjs";
import os from "../node/os.mjs";
import path from "../node/path.mjs";
import perfHooks from "../node/perf-hooks.mjs";
import punycode from "../node/punycode.mjs";
import querystring from "../node/querystring.mjs";
import readline from "../node/readline.mjs";
import readlinePromises from "../node/readline-promises.mjs";
import repl from "../node/repl.mjs";
import stringDecoder from "../node/string-decoder.mjs";
import nodeTimers from "../node/timers.mjs";
import tls from "../node/tls.mjs";
import tty from "../node/tty.mjs";
import streamWeb from "../node/stream-web.mjs";
import sqlite from "../node/sqlite.mjs";
import test from "../node/test.mjs";
import traceEvents from "../node/trace-events.mjs";
import internalStreamDuplex from "../node/internal-stream-duplex.mjs";
import internalStreamPassthrough from "../node/internal-stream-passthrough.mjs";
import internalStreamReadable from "../node/internal-stream-readable.mjs";
import internalStreamTransform from "../node/internal-stream-transform.mjs";
import internalStreamWrap from "../node/internal-stream-wrap.mjs";
import internalStreamWritable from "../node/internal-stream-writable.mjs";
import internalTlsCommon from "../node/internal-tls-common.mjs";
import internalTlsWrap from "../node/internal-tls-wrap.mjs";
import nodeUrl from "../node/url.mjs";
import util from "../node/util.mjs";
import v8 from "../node/v8.mjs";
import vm from "../node/vm.mjs";
import workerThreads from "../node/worker-threads.mjs";
import wasi from "../node/wasi.mjs";
import zlib from "../node/zlib.mjs";

installWebGlobals();
installWebSocketGlobals();
globalThis.global = globalThis;
delete globalThis.InternalError;
installProcessGlobals({
  assert,
  "assert/strict": assert,
  async_hooks: asyncHooks,
  buffer,
  child_process: childProcess,
  cluster,
  console: nodeConsole,
  constants,
  crypto: nodeCrypto,
  diagnostics_channel: diagnosticsChannel,
  dns,
  "dns/promises": dnsPromises,
  dgram,
  domain,
  events: EventEmitter,
  http,
  http2,
  inspector,
  "inspector/promises": inspector,
  _http_agent: internalHttpAgent,
  _http_client: internalHttpClient,
  _http_common: internalHttpCommon,
  _http_incoming: internalHttpIncoming,
  _http_outgoing: internalHttpOutgoing,
  _http_server: internalHttpServer,
  https,
  module: nodeModule,
  net,
  os,
  path,
  "path/posix": path.posix,
  "path/win32": path.win32,
  perf_hooks: perfHooks,
  punycode,
  stream: streams,
  process,
  fs,
  "fs/promises": fsPromises,
  querystring,
  "stream/consumers": streamConsumers,
  "stream/promises": streamPromises,
  "stream/web": streamWeb,
  _stream_duplex: internalStreamDuplex,
  _stream_passthrough: internalStreamPassthrough,
  _stream_readable: internalStreamReadable,
  _stream_transform: internalStreamTransform,
  _stream_wrap: internalStreamWrap,
  _stream_writable: internalStreamWritable,
  string_decoder: stringDecoder,
  sys: util,
  timers: nodeTimers,
  "timers/promises": nodeTimers.promises,
  tls,
  _tls_common: internalTlsCommon,
  _tls_wrap: internalTlsWrap,
  tty,
  url: nodeUrl,
  util,
  "util/types": util.types,
  zlib,
  readline,
  "readline/promises": readlinePromises,
  repl,
  v8,
  vm,
  worker_threads: workerThreads,
  sqlite,
  test,
  trace_events: traceEvents,
  wasi,
});
installConsoleGlobal();
const waitUntilValues = [];
class ExecutionContext {
  waitUntil(value) { waitUntilValues.push(Promise.resolve(value)); }
  abort() { throw new Error("The Worker isolate was aborted"); }
  passThroughOnException() {}
}
Object.defineProperty(globalThis, "__tokamak_drain_wait_until", {
  configurable: false,
  enumerable: false,
  value: async () => {
    while (waitUntilValues.length) {
      await Promise.allSettled(waitUntilValues.splice(0));
    }
  },
});
globalThis.__tokamak_context = Object.assign(Object.create(ExecutionContext.prototype), {
  access: undefined,
  cache: undefined,
  exports: undefined,
  props: {},
  tracing: createTracing(),
});
