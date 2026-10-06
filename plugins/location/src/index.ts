import { FrontendPlugin } from "@tokamakdev/plugin";
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

export type PositionCallback = (position: Position) => void;
export type PositionErrorCallback = (error: DOMException) => void;

class Location extends FrontendPlugin {
  constructor() {
    super("location");
  }

  getCurrentPosition(): Promise<Position> {
    return this.hasNativeTransport ? this.call("getCurrentPosition") : web.getCurrentPosition();
  }

  watchPosition(next: PositionCallback, error: PositionErrorCallback): () => void {
    if (!this.hasNativeTransport) return web.watchPosition(next, error);
    return this.listen("watchPosition", next, error);
  }
}

export const location = new Location();
