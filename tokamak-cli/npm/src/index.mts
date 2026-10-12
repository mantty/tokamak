/** A setting value. */
export type Value = string | number | boolean;

/** One platform's settings: the shared keys and the settings its platform pack declares. */
export interface PlatformConfig {
  /** Display name. */
  name?: string;
  /** Application identifier. */
  identifier?: string;
  /** Icon in the platform's format, relative to the Vite root. */
  icon?: string;
  /** A setting the platform pack declares; `tok` validates it. */
  [key: string]: Value | undefined;
}

/** One plugin's settings: values for every platform, and each platform's own. */
export interface PluginConfig {
  android?: Record<string, Value | undefined>;
  ios?: Record<string, Value | undefined>;
  macos?: Record<string, Value | undefined>;
  windows?: Record<string, Value | undefined>;
  /** A setting the plugin declares; `tok` validates it. */
  [key: string]: Value | Record<string, Value | undefined> | undefined;
}

/**
 * The app's tokamak configuration: the tokamak Vite plugin's options other than `module`. Other
 * keys are plugins' settings, by plugin ID.
 */
export interface Config {
  /** Display name for every platform. */
  name?: string;
  /** Application identifier for every platform. */
  identifier?: string;
  /** Icon for every platform, relative to the Vite root. */
  icon?: string;
  /** App version. */
  version?: string;
  android?: PlatformConfig;
  ios?: PlatformConfig;
  macos?: PlatformConfig;
  windows?: PlatformConfig;
  /** A plugin's settings, by its ID. */
  [plugin: string]: string | PlatformConfig | PluginConfig | undefined;
}
