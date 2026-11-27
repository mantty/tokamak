export { TokamakEntrypoint, platform } from "@tokamakdev/plugin/worker";
export type { StartEvent, TokamakContext, WebView } from "@tokamakdev/plugin/worker";
export { lifecycleStage as getLifecycleStage, type LifecycleStage } from "@tokamakdev/plugin";

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
  /** Splash image: a PNG, JPEG or GIF file, relative to the Vite root. */
  "splash-image"?: string;
  /** Splash image in dark mode, relative to the Vite root. Defaults to `splash-image`. */
  "splash-image-dark"?: string;
  /** Splash image width in points, dp or device-independent pixels. Defaults to 100. */
  "splash-width"?: number;
  /** Splash background colour, `#RRGGBB`. Defaults to `#FFFFFF`. */
  "splash-background"?: string;
  /** Splash background colour in dark mode, `#RRGGBB`. Defaults to `splash-background`. */
  "splash-background-dark"?: string;
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

/** Whether device backups include a store. */
export type Backup = "included" | "excluded";

/** A KV namespace on the device. */
export interface KvBinding {
  type: "kv";
  /** Whether device backups include the namespace. Defaults to `included`. */
  backup?: Backup;
}

/** A D1 database on the device. */
export interface D1Binding {
  type: "d1";
  /** Whether device backups include the database. Defaults to `included`. */
  backup?: Backup;
}

/** An R2 bucket on the device. */
export interface R2Binding {
  type: "r2";
  /** Whether device backups include the bucket. Defaults to `included`. */
  backup?: Backup;
}

/** A resource the app provisions on the device for the Wrangler binding of the same name. */
export type Binding = KvBinding | D1Binding | R2Binding;

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
  /** Splash image: a PNG, JPEG or GIF file, relative to the Vite root. */
  "splash-image"?: string;
  /** Splash image in dark mode, relative to the Vite root. Defaults to `splash-image`. */
  "splash-image-dark"?: string;
  /** Splash image width in points, dp or device-independent pixels. Defaults to 100. */
  "splash-width"?: number;
  /** Splash background colour, `#RRGGBB`. Defaults to `#FFFFFF`. */
  "splash-background"?: string;
  /** Splash background colour in dark mode, `#RRGGBB`. Defaults to `splash-background`. */
  "splash-background-dark"?: string;
  /** App version. */
  version?: string;
  /**
   * The resources the app provisions on the device, by the name of the Wrangler binding that binds
   * each. Every KV, D1 and R2 binding in the Wrangler configuration needs an entry.
   */
  bindings?: Record<string, Binding>;
  android?: PlatformConfig;
  ios?: PlatformConfig;
  macos?: PlatformConfig;
  windows?: PlatformConfig;
  /** A plugin's settings, by its ID. */
  [plugin: string]: string | number | PlatformConfig | PluginConfig | Record<string, Binding> | undefined;
}
