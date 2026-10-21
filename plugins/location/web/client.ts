import { valueStream, type JsonStream } from "@tokamakdev/plugin/streams";
import * as web from "./index.js";
import type { Position, PositionOptions } from "../src/types.js";

export const location = {
  getCurrentPosition: (options: PositionOptions = {}) => web.getCurrentPosition(options),
  permission: web.permission,
  watchPosition(): Promise<JsonStream<Position>> {
    return Promise.resolve(valueStream<Position>((next, error) => web.watchPosition(next, error), "dropOldest"));
  },
};
