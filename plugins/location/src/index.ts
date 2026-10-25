import { capabilitiesOf, lifecycleStage, platformApi, type JsonStream, type TokamakContext } from "@tokamakdev/plugin";

import { android } from "./android.generated.js";
import { apple } from "./apple.generated.js";
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

export function location(ctx: TokamakContext) {
  const native = platformApi(ctx, { apple, android });

  /** Requires location services and in-use permission, asking for it where the user has not answered. */
  async function authorize(): Promise<void> {
    if (!await native.servicesEnabled()) throw unavailable("Location services are disabled");
    let { inUse } = await native.permission();
    if (inUse === "prompt") ({ inUse } = await native.requestPermission());
    if (inUse !== "granted") throw permissionDenied();
  }

  return {
    /** Whether each operation is implemented where the app runs, without calling it. Permission is not reported. */
    get capabilities(): Readonly<Record<string, boolean>> {
      return capabilitiesOf(ctx, "location");
    },

    /**
     * Applies long-term location awareness, replacing the previous configuration, which the plugin
     * keeps and applies again at each start. Rejects on Apple platforms with `NotSupportedError` where
     * the device cannot monitor significant changes.
     */
    setup: (config: SetupConfig): Promise<void> => native.setup(config),

    /**
     * The device's position, asking for in-use permission where the user has not answered. Rejects
     * with `NotAllowedError` without it, `TimeoutError` after `timeout`, and `NotReadableError`
     * when location is unavailable. Asking off screen rejects with `NeedsUIError`, as
     * `requestPermission` does.
     */
    async getCurrentPosition({ maximumAge = 0, timeout }: PositionOptions = {}): Promise<Position> {
      await authorize();
      // JSON has no Infinity, which accepts any recent position.
      const recent = await native.recentPosition(Math.min(maximumAge, Number.MAX_VALUE));
      if (recent) return recent;
      const located = new AbortController();
      const timer = timeout === undefined ? undefined : setTimeout(() => { located.abort(timedOut()); }, timeout);
      const position = await platformApi(ctx, { apple, android }, located.signal).currentPosition()
        .finally(() => { clearTimeout(timer); });
      if (position) return position;
      // Android finds no position in the background without permission at all times.
      const background = await lifecycleStage(ctx) === "background";
      throw background && (await native.permission()).backgroundAlways !== "granted" ? backgroundDenied() : unavailable("Location is unavailable");
    },

    /**
     * Positions as the device moves, after `getCurrentPosition`'s permission checks, which can reject
     * with `NeedsUIError` off screen. A slow reader receives the newest.
     */
    async watchPosition(): Promise<JsonStream<Position>> {
      await authorize();
      return native.watchPosition();
    },

    /** The app's location permission, and the precision it gets. Never asks. */
    permission: (): Promise<LocationPermission> => native.permission(),

    /**
     * Asks for in-use permission, and for what `options` names, where the user has not answered.
     * Android asks for permission at all times once in-use permission is granted. Rejects with
     * `NeedsUIError` when a prompt would show off screen.
     */
    requestPermission: (options?: PermissionRequest): Promise<LocationPermission> => native.requestPermission(options),

    /** Whether location services are on for the device. While they are off, nothing locates. */
    servicesEnabled: (): Promise<boolean> => native.servicesEnabled(),

    /**
     * Opens the app's page in the system settings, where the user changes its location permission.
     * Rejects with `NeedsUIError` off screen on iOS and Android.
     */
    openSettings: (): Promise<void> => native.openSettings(),

    /**
     * Starts continuous tracking, replacing any running, until `stopUpdates`; `setup` gives long-term
     * awareness instead. The updates' positions reach `onLocationUpdate`, including after the system
     * ended the app. Rejects with `NotAllowedError` without in-use permission, and on iOS with
     * `NeedsUIError` off screen. Android's `foregroundService` rejects with `NotSupportedError`
     * without the `background-updates` build setting, and with `NeedsUIError` when Android does not
     * allow the service to start off screen.
     */
    startUpdates: (options?: UpdateOptions): Promise<void> => native.startUpdates(options),

    stopUpdates: (): Promise<void> => native.stopUpdates(),

    /** The options of the running updates, or null when none are running. */
    getUpdates: (): Promise<UpdateOptions | null> => native.getUpdates(),

    /**
     * Monitors a circular region, replacing one with the same `id`, until `removeRegion`. Its entries
     * and exits reach `onRegionTransition`, including when the system launches the app for one. A
     * region starts outside, so a device already inside reports an entry. Rejects with
     * `NotAllowedError` without precise location, and with `QuotaExceededError` beyond Apple's 20.
     */
    addRegion: (region: Region): Promise<void> => native.addRegion(region),

    /** The monitored regions. */
    getRegions: (): Promise<MonitoredRegion[]> => native.getRegions(),

    removeRegion: (id: string): Promise<void> => native.removeRegion(id),

    /** Receives the updates the Worker's `onLocationUpdate` receives, while the listening page or Worker runs. */
    onLocationUpdate: (): JsonStream<LocationUpdate> => native.onLocationUpdate(),

    /** Receives the transitions the Worker's `onRegionTransition` receives, while the listening page or Worker runs. */
    onRegionTransition: (): JsonStream<RegionTransition> => native.onRegionTransition(),

    /**
     * Receives the visits the Worker's `onVisit` receives, while the listening page or Worker runs.
     * @platforms ios macos
     */
    onVisit: (): JsonStream<Visit> => apple(ctx).onVisit(),

    /** Receives the failures the Worker's `onLocationError` receives, while the listening page or Worker runs. */
    onLocationError: (): JsonStream<LocationErrorEvent> => native.onLocationError(),
  };
}
