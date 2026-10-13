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

/** A plugin setting's value: one value, a list, or a map, of the kind the plugin declares. */
export type PluginValue = Value | Value[] | { [key: string]: Value };

/** One plugin's settings: values for every platform, and each platform's own. */
export interface PluginConfig {
  android?: Record<string, PluginValue | undefined>;
  ios?: Record<string, PluginValue | undefined>;
  macos?: Record<string, PluginValue | undefined>;
  windows?: Record<string, PluginValue | undefined>;
  /** A setting the plugin declares; `tok` validates it. */
  [key: string]: PluginValue | Record<string, PluginValue | undefined> | undefined;
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
