import { Plugin } from "@tokamakdev/plugin";
import * as web from "../web/index.js";
import { nonEmptyText } from "./checks.js";
import { monitoredRegion, validatePermissionRequest, validateSetup, validateUpdateOptions } from "./options.js";
import type {
  ErrorListener,
  Listener,
  LocationErrorEvent,
  LocationPermission,
  LocationUpdate,
  MonitoredRegion,
  PermissionRequest,
  Position,
  PositionCallback,
  PositionErrorCallback,
  PositionOptions,
  Region,
  RegionTransition,
  SetupConfig,
  UpdateOptions,
  Visit,
} from "./types.js";

export type * from "./types.js";

class Location extends Plugin {
  constructor() {
    super("location");
  }

  /**
   * Applies what runs at every launch, replacing the previous configuration, which the plugin keeps
   * and applies again at each start. Rejects with `TypeError`, changing nothing, when a value is
   * invalid.
   */
  async setup(config: SetupConfig): Promise<void> {
    validateSetup(config, "config");
    return this.hasNativeTransport ? this.call("setup", config) : web.unsupported();
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

  /**
   * Starts continuous updates, replacing any running, until `stopUpdates`. Their positions reach
   * `onLocationUpdate`, including after the system ended the app. Rejects with `NotAllowedError`
   * without in-use permission, and on iOS with `NeedsUIError` off screen. Android's
   * `foregroundService` rejects with `NotSupportedError` without the `background-updates` build
   * setting, and with `NeedsUIError` when Android does not allow the service to start off screen.
   */
  async startUpdates(options: UpdateOptions = {}): Promise<void> {
    validateUpdateOptions(options, "options");
    return this.hasNativeTransport ? this.call("startUpdates", options) : web.unsupported();
  }

  stopUpdates(): Promise<void> {
    return this.hasNativeTransport ? this.call("stopUpdates") : web.unsupported();
  }

  /** The options of the running updates, or null when none are running. */
  getUpdates(): Promise<UpdateOptions | null> {
    return this.hasNativeTransport ? this.call("getUpdates") : web.unsupported();
  }

  /**
   * Monitors a circular region, replacing one with the same `id`, until `removeRegion`. Its entries
   * and exits reach `onRegionTransition`, including when the system launches the app for one. A
   * region starts outside, so a device already inside reports an entry. Rejects with
   * `NotAllowedError` without precise location, and with `QuotaExceededError` beyond Apple's 20.
   */
  async addRegion(region: Region): Promise<void> {
    const monitored = monitoredRegion(region);
    return this.hasNativeTransport ? this.call("addRegion", monitored) : web.unsupported();
  }

  /** The monitored regions. */
  getRegions(): Promise<MonitoredRegion[]> {
    return this.hasNativeTransport ? this.call("getRegions") : web.unsupported();
  }

  async removeRegion(id: string): Promise<void> {
    nonEmptyText(id, "id");
    return this.hasNativeTransport ? this.call("removeRegion", { id }) : web.unsupported();
  }

  /** Receives the updates the Worker's `onLocationUpdate` receives, while the listening page or Worker runs. */
  onLocationUpdate(listener: Listener<LocationUpdate>, error: ErrorListener = report): () => void {
    return this.hasNativeTransport ? this.listen("onLocationUpdate", listener, error) : () => undefined;
  }

  /** Receives the transitions the Worker's `onRegionTransition` receives, while the listening page or Worker runs. */
  onRegionTransition(listener: Listener<RegionTransition>, error: ErrorListener = report): () => void {
    return this.hasNativeTransport ? this.listen("onRegionTransition", listener, error) : () => undefined;
  }

  /** Apple only: receives the visits the Worker's `onVisit` receives, while the listening page or Worker runs. */
  onVisit(listener: Listener<Visit>, error: ErrorListener = report): () => void {
    return this.hasNativeTransport ? this.listen("onVisit", listener, error) : () => undefined;
  }

  /** Receives the failures the Worker's `onLocationError` receives, while the listening page or Worker runs. */
  onLocationError(listener: Listener<LocationErrorEvent>, error: ErrorListener = report): () => void {
    return this.hasNativeTransport ? this.listen("onLocationError", listener, error) : () => undefined;
  }
}

function report(error: DOMException): void {
  console.error(error);
}

const unlimited = 0xffffffff;

/** `value` clamped to whole milliseconds, as the Geolocation API reads its options. */
function milliseconds(value: number): number {
  return Number.isNaN(value) ? 0 : Math.min(Math.max(Math.round(value), 0), unlimited);
}

export const location = new Location();
