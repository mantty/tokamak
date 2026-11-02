// The location plugin's declarations, from its build settings: the Info.plist on Apple platforms
// and the Android manifest.

/** The location plugin's build settings, with their defaults applied. */
export interface Settings {
  "background-always": boolean;
  "background-updates": boolean;
  precision: "precise" | "approximate";
  "when-in-use-description"?: string;
  "always-description"?: string;
  "accuracy-purposes"?: Record<string, string>;
}

/**
 * The file that declares what the settings ask for, the plugin section key naming it, the Swift
 * compilation conditions of an Apple build, and notices.
 */
export interface Declarations {
  key: "plist" | "manifest";
  name: string;
  contents: string;
  notices: string[];
  conditions?: string[];
}

/** A property list value. */
type PlistValue = boolean | string | PlistValue[] | { [key: string]: PlistValue };

const WHEN_IN_USE = "This app uses your location when you request location features.";
const ALWAYS = "This app uses your location in the background for location features you turn on.";

/**
 * The file that declares what `settings` ask for on `platform`, the plugin section key that names
 * it, the Swift compilation conditions of an Apple build, and notices for the build; or the
 * problems with the settings.
 */
export function declarations(platform: string, settings: Settings): Declarations | { problems: string[] } {
  if (settings["always-description"] !== undefined && !settings["background-always"]) {
    return { problems: ["always-description needs background-always, which declares the permission it describes"] };
  }
  if (platform === "android") {
    return { key: "manifest", name: "AndroidManifest.xml", contents: androidManifest(settings), notices: [] };
  }
  const notices =
    platform === "macos" && settings.precision === "approximate"
      ? ["macOS has no declaration of approximate location, so precision approximate applies to iOS and Android only"]
      : [];
  const keys = appleKeys(platform, settings);
  // Only a build that gives the Always reason compiles the code that asks for Always.
  const always = "NSLocationAlwaysAndWhenInUseUsageDescription" in keys;
  return {
    key: "plist",
    name: "Info.plist",
    contents: plist(keys),
    notices,
    ...(always ? { conditions: ["LOCATION_ALWAYS"] } : {}),
  };
}

/** The Info.plist keys of `settings` on the Apple `platform`. */
function appleKeys(platform: string, settings: Settings): Record<string, PlistValue> {
  const whenInUse = settings["when-in-use-description"] ?? WHEN_IN_USE;
  const purposes = settings["accuracy-purposes"] ?? {};
  const temporary: Record<string, PlistValue> =
    Object.keys(purposes).length > 0 ? { NSLocationTemporaryUsageDescriptionDictionary: purposes } : {};
  if (platform === "macos") {
    // macOS reads NSLocationUsageDescription, and its SDK headers name the iOS keys.
    return {
      NSLocationUsageDescription: whenInUse,
      NSLocationWhenInUseUsageDescription: whenInUse,
      NSLocationAlwaysAndWhenInUseUsageDescription: whenInUse,
      ...temporary,
    };
  }
  const keys: Record<string, PlistValue> = { NSLocationWhenInUseUsageDescription: whenInUse };
  if (settings["background-always"]) keys.NSLocationAlwaysAndWhenInUseUsageDescription = settings["always-description"] ?? ALWAYS;
  if (settings["background-updates"]) keys.UIBackgroundModes = ["location"];
  if (settings.precision === "approximate") keys.NSLocationDefaultAccuracyReduced = true;
  return { ...keys, ...temporary };
}

/** The Android manifest of `settings`. */
function androidManifest(settings: Settings): string {
  const permissions = ["ACCESS_COARSE_LOCATION", "RECEIVE_BOOT_COMPLETED"];
  if (settings.precision !== "approximate") permissions.push("ACCESS_FINE_LOCATION");
  if (settings["background-always"]) permissions.push("ACCESS_BACKGROUND_LOCATION");
  if (settings["background-updates"]) permissions.push("FOREGROUND_SERVICE", "FOREGROUND_SERVICE_LOCATION");
  const service = settings["background-updates"]
    ? [
        '    <service android:name="com.tokamak.plugins.location.TokamakLocationService" android:exported="false"',
        '        android:foregroundServiceType="location" />',
      ]
    : [];
  return [
    '<manifest xmlns:android="http://schemas.android.com/apk/res/android">',
    ...permissions.map((name) => `  <uses-permission android:name="android.permission.${name}" />`),
    "  <application>",
    ...service,
    '    <receiver android:name="com.tokamak.plugins.location.TokamakLocationReceiver" android:exported="false" />',
    '    <receiver android:name="com.tokamak.plugins.location.TokamakLocationBootReceiver" android:exported="false">',
    "      <intent-filter>",
    '        <action android:name="android.intent.action.BOOT_COMPLETED" />',
    '        <action android:name="android.intent.action.MY_PACKAGE_REPLACED" />',
    "      </intent-filter>",
    "    </receiver>",
    "  </application>",
    "</manifest>",
    "",
  ].join("\n");
}

/** `keys` as an XML property list. */
function plist(keys: Record<string, PlistValue>): string {
  return [
    '<?xml version="1.0" encoding="UTF-8"?>',
    '<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">',
    '<plist version="1.0">',
    ...element(keys, ""),
    "</plist>",
    "",
  ].join("\n");
}

/** The lines of `value` as a property list element, indented by `indent`. */
function element(value: PlistValue, indent: string): string[] {
  if (typeof value === "boolean") return [`${indent}<${String(value)}/>`];
  if (typeof value === "string") return [`${indent}<string>${escaped(value)}</string>`];
  const inner = `${indent}\t`;
  if (Array.isArray(value)) {
    return [`${indent}<array>`, ...value.flatMap((item) => element(item, inner)), `${indent}</array>`];
  }
  const entries = Object.entries(value).flatMap(([key, item]) => [
    `${inner}<key>${escaped(key)}</key>`,
    ...element(item, inner),
  ]);
  return [`${indent}<dict>`, ...entries, `${indent}</dict>`];
}

function escaped(text: string): string {
  return text.replaceAll("&", "&amp;").replaceAll("<", "&lt;").replaceAll(">", "&gt;");
}
