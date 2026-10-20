import { valueStream } from "@tokamakdev/plugin/streams";
import * as web from "./index.js";
import type { Position, PositionOptions } from "../src/types.js";

export const location = {
  getCurrentPosition: (options: PositionOptions = {}) => web.getCurrentPosition(options),
  permission: web.permission,
  watchPosition(): Promise<ReadableStream<Uint8Array>> {
    return Promise.resolve(valueStream<Position>((next, error) => web.watchPosition(next, error)));
  },
};
