import { Plugin, type JsonStream, type TokamakContext } from "@tokamakdev/plugin";
import { nonEmptyText } from "./checks.js";
import { monitoredRegion, validatePermissionRequest, validateSetup, validateUpdateOptions } from "./options.js";
import type {
  LocationErrorEvent,
  LocationPermission,
  LocationUpdate,
  MonitoredRegion,
  PermissionRequest,
  Position,
  PositionOptions,
  Region,
  RegionTransition,
  SetupConfig,
  UpdateOptions,
  Visit,
} from "./types.js";

export type * from "./types.js";

class Location extends Plugin {
  constructor(ctx: TokamakContext) {
    super(ctx, "location");
  }

  /**
   * Applies long-term location awareness, replacing the previous configuration, which the plugin keeps
   * and applies again at each start. Rejects with `TypeError`, changing nothing, when a value is
   * invalid, and on Apple platforms with `NotSupportedError` where the device cannot monitor
   * significant changes.
   */
  async setup(config: SetupConfig): Promise<void> {
    validateSetup(config, "config");
    return this.call("setup", config);
  }

  getCurrentPosition(options: PositionOptions = {}): Promise<Position> {
    return this.call("getCurrentPosition", {
      maximumAge: milliseconds(options.maximumAge ?? 0),
      timeout: milliseconds(options.timeout ?? unlimited),
    });
  }

  /** Positions as the device moves. A slow reader receives the newest. */
  watchPosition(): JsonStream<Position> {
    return this.stream("watchPosition", null, "dropOldest");
  }

  /** The app's in-use and background permission, and the precision it gets. Never asks. */
  permission(): Promise<LocationPermission> {
    return this.call("permission");
  }

  /**
   * Asks for in-use permission, and for what `options` names, where the user has not answered.
   * Android asks for background permission once in-use permission is granted. Rejects with
   * `NeedsUIError` when a prompt would show off screen.
   */
  async requestPermission(options: PermissionRequest = {}): Promise<LocationPermission> {
    validatePermissionRequest(options, "options");
    return this.call("requestPermission", options);
  }

  /** Whether location services are on for the device. While they are off, nothing locates. */
  servicesEnabled(): Promise<boolean> {
    return this.call("servicesEnabled");
  }

  /** Opens the app's page in the system settings, where the user changes its location permission. */
  openSettings(): Promise<void> {
    return this.call("openSettings");
  }

  /**
   * Starts continuous tracking, replacing any running, until `stopUpdates`; `setup` gives long-term
   * awareness instead. The updates' positions reach `onLocationUpdate`, including after the system
   * ended the app. Rejects with `NotAllowedError` without in-use permission, and on iOS with
   * `NeedsUIError` off screen. Android's `foregroundService` rejects with `NotSupportedError`
   * without the `background-updates` build setting, and with `NeedsUIError` when Android does not
   * allow the service to start off screen.
   */
  async startUpdates(options: UpdateOptions = {}): Promise<void> {
    validateUpdateOptions(options, "options");
    return this.call("startUpdates", options);
  }

  stopUpdates(): Promise<void> {
    return this.call("stopUpdates");
  }

  /** The options of the running updates, or null when none are running. */
  getUpdates(): Promise<UpdateOptions | null> {
    return this.call("getUpdates");
  }

  /**
   * Monitors a circular region, replacing one with the same `id`, until `removeRegion`. Its entries
   * and exits reach `onRegionTransition`, including when the system launches the app for one. A
   * region starts outside, so a device already inside reports an entry. Rejects with
   * `NotAllowedError` without precise location, and with `QuotaExceededError` beyond Apple's 20.
   */
  async addRegion(region: Region): Promise<void> {
    const monitored = monitoredRegion(region);
    return this.call("addRegion", monitored);
  }

  /** The monitored regions. */
  getRegions(): Promise<MonitoredRegion[]> {
    return this.call("getRegions");
  }

  async removeRegion(id: string): Promise<void> {
    nonEmptyText(id, "id");
    return this.call("removeRegion", { id });
  }

  /** Receives the updates the Worker's `onLocationUpdate` receives, while the listening page or Worker runs. */
  onLocationUpdate(): JsonStream<LocationUpdate> {
    return this.stream("onLocationUpdate", null, "dropOldest");
  }

  /** Receives the transitions the Worker's `onRegionTransition` receives, while the listening page or Worker runs. */
  onRegionTransition(): JsonStream<RegionTransition> {
    return this.stream("onRegionTransition");
  }

  /** Apple only: receives the visits the Worker's `onVisit` receives, while the listening page or Worker runs. */
  onVisit(): JsonStream<Visit> {
    return this.stream("onVisit");
  }

  /** Receives the failures the Worker's `onLocationError` receives, while the listening page or Worker runs. */
  onLocationError(): JsonStream<LocationErrorEvent> {
    return this.stream("onLocationError");
  }
}


const unlimited = 0xffffffff;

/** `value` clamped to whole milliseconds, as the Geolocation API reads its options. */
function milliseconds(value: number): number {
  return Number.isNaN(value) ? 0 : Math.min(Math.max(Math.round(value), 0), unlimited);
}

export function location(ctx: TokamakContext): Location { return new Location(ctx); }
