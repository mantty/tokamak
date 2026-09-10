import EventEmitter from "../events/events.mjs";
import { installConsoleGlobal } from "../globals/console.mjs";
import process, { installProcessGlobals } from "../globals/process.mjs";
import { installWebGlobals } from "../globals/web.mjs";
import { installWebSocketGlobals } from "../network/websocket.mjs";
import streams from "../streams/node.mjs";
import fs from "node:fs";
import fsPromises from "node:fs/promises";

installWebGlobals();
installWebSocketGlobals();
installProcessGlobals({
  events: EventEmitter,
  stream: streams,
  process,
  fs,
  "fs/promises": fsPromises,
});
installConsoleGlobal();
globalThis.__tokamak_context = {
  __waitUntil: [],
  waitUntil(value) { this.__waitUntil.push(Promise.resolve(value)); },
  passThroughOnException() { throw new Error("passThroughOnException is not supported by tokamak"); },
};
