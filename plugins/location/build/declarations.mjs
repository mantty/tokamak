// The location plugin's declarations, from its build settings: the Info.plist on Apple platforms
// and the Android manifest.

const WHEN_IN_USE = "This app uses your location when you request location features.";
const ALWAYS = "This app uses your location in the background for location features you turn on.";

/**
 * The file that declares what `settings` ask for on `platform`, the plugin section key that names
 * it, and notices for the build; or the problems with the settings.
 */
export function declarations(platform, settings) {
  if (settings["always-description"] !== undefined && !settings.background) {
    return { problems: ["always-description needs background, which declares the permission it describes"] };
  }
  if (platform === "android") {
    return { key: "manifest", name: "AndroidManifest.xml", contents: androidManifest(settings), notices: [] };
  }
  const notices =
    platform === "macos" && settings.precision === "approximate"
      ? ["macOS has no declaration of approximate location, so precision approximate applies to iOS and Android only"]
      : [];
  return { key: "plist", name: "Info.plist", contents: plist(appleKeys(platform, settings)), notices };
}

/** The Info.plist keys of `settings` on the Apple `platform`. */
function appleKeys(platform, settings) {
  const whenInUse = settings["when-in-use-description"] ?? WHEN_IN_USE;
  // macOS reads only NSLocationUsageDescription; its SDK headers still name the iOS keys.
  const keys =
    platform === "macos"
      ? {
          NSLocationUsageDescription: whenInUse,
          NSLocationWhenInUseUsageDescription: whenInUse,
          NSLocationAlwaysAndWhenInUseUsageDescription: whenInUse,
        }
      : { NSLocationWhenInUseUsageDescription: whenInUse };
  if (platform === "ios" && settings.background) {
    keys.NSLocationAlwaysAndWhenInUseUsageDescription = settings["always-description"] ?? ALWAYS;
  }
  if (platform === "ios" && settings["background-updates"]) keys.UIBackgroundModes = ["location"];
  if (platform === "ios" && settings.precision === "approximate") keys.NSLocationDefaultAccuracyReduced = true;
  const purposes = settings["accuracy-purposes"] ?? {};
  if (Object.keys(purposes).length > 0) keys.NSLocationTemporaryUsageDescriptionDictionary = purposes;
  return keys;
}

/** The Android manifest of `settings`. */
function androidManifest(settings) {
  const permissions = ["ACCESS_COARSE_LOCATION", "RECEIVE_BOOT_COMPLETED"];
  if (settings.precision !== "approximate") permissions.push("ACCESS_FINE_LOCATION");
  if (settings.background) permissions.push("ACCESS_BACKGROUND_LOCATION");
  return [
    '<manifest xmlns:android="http://schemas.android.com/apk/res/android">',
    ...permissions.map((name) => `  <uses-permission android:name="android.permission.${name}" />`),
    "  <application>",
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
function plist(keys) {
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
function element(value, indent) {
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

function escaped(text) {
  return text.replaceAll("&", "&amp;").replaceAll("<", "&lt;").replaceAll(">", "&gt;");
}
