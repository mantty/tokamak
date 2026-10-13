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
