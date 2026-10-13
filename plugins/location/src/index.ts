import { Plugin } from "@tokamakdev/plugin";
import * as web from "../web/index.js";
import { validatePermissionRequest } from "./options.js";
import type {
  LocationPermission,
  PermissionRequest,
  Position,
  PositionCallback,
  PositionErrorCallback,
  PositionOptions,
} from "./types.js";

export type * from "./types.js";

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

  /** The app's in-use and background permission, and the precision it gets. Never asks. */
  permission(): Promise<LocationPermission> {
    return this.hasNativeTransport ? this.call("permission") : web.permission();
  }

  /**
   * Asks for in-use permission, and for what `options` names, where the user has not answered.
   * Android asks for background permission once in-use permission is granted. Rejects with
   * `NeedsUIError` when a prompt would show off screen.
   */
  async requestPermission(options: PermissionRequest = {}): Promise<LocationPermission> {
    validatePermissionRequest(options, "options");
    return this.hasNativeTransport ? this.call("requestPermission", options) : web.unsupported();
  }

  /** Whether location services are on for the device. While they are off, nothing locates. */
  servicesEnabled(): Promise<boolean> {
    return this.hasNativeTransport ? this.call("servicesEnabled") : web.unsupported();
  }

  /** Opens the app's page in the system settings, where the user changes its location permission. */
  openSettings(): Promise<void> {
    return this.hasNativeTransport ? this.call("openSettings") : web.unsupported();
  }
}

const unlimited = 0xffffffff;

/** `value` clamped to whole milliseconds, as the Geolocation API reads its options. */
function milliseconds(value: number): number {
  return Number.isNaN(value) ? 0 : Math.min(Math.max(Math.round(value), 0), unlimited);
}

export const location = new Location();
