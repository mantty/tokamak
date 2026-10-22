import { Plugin, lifecycleStage, type JsonStream, type TokamakContext } from "@tokamakdev/plugin";

import { backgroundDenied, permissionDenied, timedOut, unavailable } from "./errors.js";
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
   * and applies again at each start. Rejects on Apple platforms with `NotSupportedError` where the
   * device cannot monitor significant changes.
   */
  setup(config: SetupConfig): Promise<void> {
    return this.call("setup", config);
  }

  /**
   * The device's position, asking for in-use permission where the user has not answered. Rejects
   * with `NotAllowedError` without it, `TimeoutError` after `timeout`, and `NotReadableError`
   * when location is unavailable. Asking off screen rejects with `NeedsUIError`, as
   * `requestPermission` does.
   */
  async getCurrentPosition({ maximumAge = 0, timeout }: PositionOptions = {}): Promise<Position> {
    await this.#authorize();
    // JSON has no Infinity, which accepts any recent position.
    const recent = await this.call<Position | null>("recentPosition", { maximumAge: Math.min(maximumAge, Number.MAX_VALUE) });
    if (recent) return recent;
    const located = new AbortController();
    const timer = timeout === undefined ? undefined : setTimeout(() => { located.abort(timedOut()); }, timeout);
    const position = await this.call<Position | null>("currentPosition", null, { signal: located.signal })
      .finally(() => { clearTimeout(timer); });
    if (position) return position;
    // Android finds no position in the background without background permission.
    const background = await lifecycleStage(this.ctx) === "background";
    throw background && (await this.permission()).background !== "granted" ? backgroundDenied() : unavailable("Location is unavailable");
  }

  /**
   * Positions as the device moves, after `getCurrentPosition`'s permission checks, which can reject
   * with `NeedsUIError` off screen. A slow reader receives the newest.
   */
  watchPosition(): JsonStream<Position> {
    return this.stream("watchPosition", this.#authorize().then(() => null), "dropOldest");
  }

  /** Requires location services and in-use permission, asking for it where the user has not answered. */
  async #authorize(): Promise<void> {
    if (!await this.servicesEnabled()) throw unavailable("Location services are disabled");
    let { inUse } = await this.permission();
    if (inUse === "prompt") ({ inUse } = await this.requestPermission());
    if (inUse !== "granted") throw permissionDenied();
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
  requestPermission(options: PermissionRequest = {}): Promise<LocationPermission> {
    return this.call("requestPermission", options);
  }

  /** Whether location services are on for the device. While they are off, nothing locates. */
  servicesEnabled(): Promise<boolean> {
    return this.call("servicesEnabled");
  }

  /**
   * Opens the app's page in the system settings, where the user changes its location permission.
   * Rejects with `NeedsUIError` off screen on iOS and Android.
   */
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
  startUpdates(options: UpdateOptions = {}): Promise<void> {
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
  addRegion(region: Region): Promise<void> {
    return this.call("addRegion", region);
  }

  /** The monitored regions. */
  getRegions(): Promise<MonitoredRegion[]> {
    return this.call("getRegions");
  }

  removeRegion(id: string): Promise<void> {
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

  /**
   * Receives the visits the Worker's `onVisit` receives, while the listening page or Worker runs.
   * @platforms ios macos
   */
  onVisit(): JsonStream<Visit> {
    return this.stream("onVisit");
  }

  /** Receives the failures the Worker's `onLocationError` receives, while the listening page or Worker runs. */
  onLocationError(): JsonStream<LocationErrorEvent> {
    return this.stream("onLocationError");
  }
}


export function location(ctx: TokamakContext): Location { return new Location(ctx); }
