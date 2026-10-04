/** A setting value. */
export type Value = string | number | boolean;

/** One platform's settings: the shared keys and the settings its platform pack declares. */
export interface PlatformConfig {
  /** Display name. */
  name?: string;
  /** Application identifier. */
  identifier?: string;
  /** Icon in the platform's format, relative to the configuration file. */
  icon?: string;
  /** A setting the platform pack declares; `tok` validates it. */
  [key: string]: Value | undefined;
}

/** The app's tokamak configuration, exported as `config` from `src/tokamak.ts`. */
export interface Config {
  /** Display name for every platform. */
  name?: string;
  /** Application identifier for every platform. */
  identifier?: string;
  /** Icon for every platform, relative to the configuration file. */
  icon?: string;
  /** App version. */
  version?: string;
  android?: PlatformConfig;
  ios?: PlatformConfig;
  macos?: PlatformConfig;
  windows?: PlatformConfig;
}
