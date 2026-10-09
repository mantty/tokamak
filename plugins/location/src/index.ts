import { Plugin } from "@tokamakdev/plugin";
import * as web from "../web/index.js";

export interface Coordinates {
  readonly latitude: number;
  readonly longitude: number;
  readonly accuracy: number;
  readonly altitude: number | null;
  readonly altitudeAccuracy: number | null;
  readonly heading: number | null;
  readonly speed: number | null;
}

export interface Position {
  readonly coords: Coordinates;
  readonly timestamp: number;
}

/** Options for `getCurrentPosition`, as the Geolocation API reads them. */
export interface PositionOptions {
  /** The age in milliseconds of a recent position to accept instead of a new one; 0 by default. */
  readonly maximumAge?: number;
  /** Milliseconds to wait for a position before rejecting with `TimeoutError`; unlimited by default. */
  readonly timeout?: number;
}

export type PositionCallback = (position: Position) => void;
export type PositionErrorCallback = (error: DOMException) => void;

class Location extends Plugin {
  constructor() {
    super("location");
  }

  getCurrentPosition(options: PositionOptions = {}): Promise<Position> {
    if (!this.hasNativeTransport) return web.getCurrentPosition(options);
    return this.call("getCurrentPosition", {
      maximumAge: milliseconds(options.maximumAge ?? 0),
      timeout: milliseconds(options.timeout ?? unlimited),
    });
  }

  watchPosition(next: PositionCallback, error: PositionErrorCallback): () => void {
    if (!this.hasNativeTransport) return web.watchPosition(next, error);
    return this.listen("watchPosition", next, error);
  }
}

const unlimited = 0xffffffff;

/** `value` clamped to whole milliseconds, as the Geolocation API reads its options. */
function milliseconds(value: number): number {
  return Number.isNaN(value) ? 0 : Math.min(Math.max(Math.round(value), 0), unlimited);
}

export const location = new Location();
