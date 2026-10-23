/** A position's coordinates, without those the device does not report. */
export interface Coordinates {
  readonly latitude: number;
  readonly longitude: number;
  readonly accuracy: number;
  readonly altitude?: number;
  readonly altitudeAccuracy?: number;
  readonly heading?: number;
  readonly speed?: number;
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

/**
 * Long-term location awareness, which runs across launches and restarts, uses little power and shows
 * no indicator: `setup` starts what it names and stops what it leaves out. Off screen it needs
 * background permission, with which the system launches the app for its events.
 */
export interface SetupConfig {
  /**
   * Delivers moves of 500 metres or more, at most every 5 minutes, to `onLocationUpdate`, located
   * from cellular and Wi-Fi rather than GPS: Apple's significant-change service, and on Android a
   * low-power request with the same thresholds, which Android limits to a few an hour off screen.
   */
  readonly significantChanges?: boolean;
  /** Apple only: delivers arrivals at and departures from places to `onVisit`. */
  readonly visits?: boolean;
}

/** Whether the user granted a permission, denied it, or has not been asked. */
export type Permission = "granted" | "denied" | "prompt";

export type Precision = "precise" | "approximate";

/** The app's location permission. */
export interface LocationPermission {
  /** Location while the app is in use. */
  readonly inUse: Permission;
  /** Location while it is not: Apple's Always, Android's "Allow all the time". The same as `inUse` on macOS. */
  readonly background: Permission;
  /** The precision the app gets, or would get once allowed. */
  readonly precision: Precision;
}

/** What `requestPermission` asks for besides in-use permission. */
export interface PermissionRequest {
  /** Background permission, which needs the `background` build setting. */
  readonly background?: boolean;
  /**
   * `"precise"` asks for precise location where the user allowed approximate: on Apple platforms
   * temporarily, for `purpose`. `"approximate"` asks Android for approximate location only.
   */
  readonly precision?: Precision;
  /** Apple only: the key in the `accuracy-purposes` build setting of the reason to show. */
  readonly purpose?: string;
}

/**
 * Apple only: the live update configuration, by Apple's name, which tunes when updates pause and
 * how coordinates follow roads. `"maritime"` needs iOS 27 or macOS 27.
 */
export type Activity =
  | "default"
  | "automotiveNavigation"
  | "otherNavigation"
  | "fitness"
  | "airborne"
  | "maritime";

/**
 * `startUpdates` options, for continuous tracking such as a run or navigation, which can show the
 * system's indicator: the blue pill on iOS with in-use permission, and the notification of
 * Android's `foregroundService`. Each option is platform-specific: Apple's relaunching updates
 * take only an `activity`.
 */
export interface UpdateOptions {
  /** Apple only: `"default"` by default. */
  readonly activity?: Activity;
  /** Android only: the request's quality, precision traded against battery use; `"high"` by default. */
  readonly accuracy?: "high" | "balanced" | "low";
  /** Android only: metres the device moves before the next update; 0 by default. */
  readonly distance?: number;
  /** Android only: milliseconds between updates; 10 seconds by default. */
  readonly interval?: number;
  /**
   * Android only: runs updates in a foreground service showing this notification, so they arrive at
   * the requested rate off screen. Needs the `background-updates` build setting.
   */
  readonly foregroundService?: ServiceNotification;
}

/** Android only: the foreground service's notification, set from JavaScript so apps can translate it. */
export interface ServiceNotification {
  readonly title: string;
  readonly body: string;
  /** The name of the notification channel, which Android's settings show. */
  readonly channel: string;
}

/** Positions from continuous updates or significant changes. */
export interface LocationUpdate {
  /**
   * Oldest first: those that arrived while the Worker handled the previous update come together. Each
   * is newer than those delivered before it, so one that several services report arrives once.
   */
  readonly positions: readonly Position[];
  /**
   * Apple only, from iOS 18 and macOS 15: true on the last update before updates pause while the
   * device is still. `positions` is empty when the update has no new position.
   */
  readonly stationary?: boolean;
}

/** A circular region to monitor, with the same fields as the notifications plugin's `Region`. */
export interface Region {
  readonly id: string;
  readonly latitude: number;
  readonly longitude: number;
  /** In metres. */
  readonly radius: number;
  /** Whether entering the region reaches `onRegionTransition`; true by default. */
  readonly onEntry?: boolean;
  /** Whether leaving it does; true by default. */
  readonly onExit?: boolean;
}

/** A monitored region. */
export interface MonitoredRegion extends Required<Region> {
  /** Apple only: whether the device is inside the region, as the system last determined. */
  readonly state?: "inside" | "outside" | "unknown";
}

/** An entry into or exit from a monitored region. */
export interface RegionTransition {
  readonly id: string;
  readonly transition: "enter" | "exit";
  /** Milliseconds since the epoch. */
  readonly timestamp: number;
}

/** Apple only: an arrival at or departure from a place where the user stays. */
export interface Visit {
  readonly latitude: number;
  readonly longitude: number;
  /** In metres. */
  readonly accuracy: number;
  /** Milliseconds since the epoch, absent when unknown. */
  readonly arrival?: number;
  /** Milliseconds since the epoch, absent before the user leaves. */
  readonly departure?: number;
}

/** A failure of a background location service. */
export interface LocationErrorEvent {
  /** A `DOMException` name, such as `"NotAllowedError"` or `"NotReadableError"`. */
  readonly name: string;
  readonly message: string;
  /** The region's id, when the failure concerns one. */
  readonly id?: string;
}

export type Listener<T> = (event: T) => void;
export type ErrorListener = (error: DOMException) => void;
