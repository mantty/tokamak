import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { test } from "node:test";

import { declarations, type Settings } from "../build/declarations.mjs";

const defaults: Settings = { background: false, "background-updates": false, precision: "precise" };

const WHEN_IN_USE = "This app uses your location when you request location features.";

/** The plist file `declarations` writes, with each key and value on one line. */
function plistLines(platform: string, settings: Partial<Settings> = {}): string[] {
  const { contents } = declarations(platform, { ...defaults, ...settings });
  return (contents ?? "").split("\n").map((line) => line.trim());
}

/** The permissions the Android manifest of `settings` requests. */
function permissions(settings: Partial<Settings> = {}): string[] {
  const { contents } = declarations("android", { ...defaults, ...settings });
  return [...(contents ?? "").matchAll(/android\.permission\.(\w+)/g)].map(([, name]) => name);
}

void test("declares in-use location on iOS by default", () => {
  assert.deepEqual(declarations("ios", defaults), {
    key: "plist",
    name: "Info.plist",
    contents: [
      '<?xml version="1.0" encoding="UTF-8"?>',
      '<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">',
      '<plist version="1.0">',
      "<dict>",
      "\t<key>NSLocationWhenInUseUsageDescription</key>",
      `\t<string>${WHEN_IN_USE}</string>`,
      "</dict>",
      "</plist>",
      "",
    ].join("\n"),
    notices: [],
  });
});

void test("declares what each iOS setting asks for", () => {
  const lines = plistLines("ios", {
    background: true,
    "background-updates": true,
    precision: "approximate",
    "when-in-use-description": "Shows nearby <shops> & cafés",
    "always-description": "Records your runs",
    "accuracy-purposes": { RunTracking: "Measures your run" },
  });

  assert.deepEqual(lines.slice(3, -2), [
    "<dict>",
    "<key>NSLocationWhenInUseUsageDescription</key>",
    "<string>Shows nearby &lt;shops&gt; &amp; cafés</string>",
    "<key>NSLocationAlwaysAndWhenInUseUsageDescription</key>",
    "<string>Records your runs</string>",
    "<key>UIBackgroundModes</key>",
    "<array>",
    "<string>location</string>",
    "</array>",
    "<key>NSLocationDefaultAccuracyReduced</key>",
    "<true/>",
    "<key>NSLocationTemporaryUsageDescriptionDictionary</key>",
    "<dict>",
    "<key>RunTracking</key>",
    "<string>Measures your run</string>",
    "</dict>",
    "</dict>",
  ]);
});

void test("uses a generic Always reason without always-description", () => {
  assert.ok(
    plistLines("ios", { background: true }).includes(
      "<string>This app uses your location in the background for location features you turn on.</string>",
    ),
  );
});

void test("declares the macOS usage key and the iOS keys it names with the same reason", () => {
  const { contents, notices } = declarations("macos", {
    ...defaults,
    background: true,
    "background-updates": true,
    "when-in-use-description": "Finds you",
    "always-description": "Unused on macOS",
  });
  const lines = (contents ?? "").split("\n").map((line) => line.trim());

  assert.deepEqual(lines.slice(3, -2), [
    "<dict>",
    "<key>NSLocationUsageDescription</key>",
    "<string>Finds you</string>",
    "<key>NSLocationWhenInUseUsageDescription</key>",
    "<string>Finds you</string>",
    "<key>NSLocationAlwaysAndWhenInUseUsageDescription</key>",
    "<string>Finds you</string>",
    "</dict>",
  ]);
  assert.deepEqual(notices, []);
});

void test("notices that macOS cannot declare approximate location", () => {
  const { notices, contents } = declarations("macos", { ...defaults, precision: "approximate" });

  assert.deepEqual(notices, [
    "macOS has no declaration of approximate location, so precision approximate applies to iOS and Android only",
  ]);
  assert.ok(!contents?.includes("NSLocationDefaultAccuracyReduced"));
});

void test("declares Android permissions for each setting", () => {
  assert.deepEqual(permissions(), ["ACCESS_COARSE_LOCATION", "RECEIVE_BOOT_COMPLETED", "ACCESS_FINE_LOCATION"]);
  assert.deepEqual(permissions({ precision: "approximate" }), ["ACCESS_COARSE_LOCATION", "RECEIVE_BOOT_COMPLETED"]);
  assert.deepEqual(permissions({ background: true }), [
    "ACCESS_COARSE_LOCATION",
    "RECEIVE_BOOT_COMPLETED",
    "ACCESS_FINE_LOCATION",
    "ACCESS_BACKGROUND_LOCATION",
  ]);
  assert.equal(declarations("android", defaults).key, "manifest");
});

void test("declares the Android receivers of updates, regions and restarts", () => {
  const { contents } = declarations("android", defaults);

  assert.match(contents ?? "", /<receiver android:name="com\.tokamak\.plugins\.location\.TokamakLocationReceiver" android:exported="false" \/>/);
  assert.match(contents ?? "", /TokamakLocationBootReceiver[\s\S]*BOOT_COMPLETED[\s\S]*MY_PACKAGE_REPLACED/);
});

void test("rejects always-description without background", () => {
  for (const platform of ["ios", "macos", "android"]) {
    assert.deepEqual(declarations(platform, { ...defaults, "always-description": "Records your runs" }), {
      problems: ["always-description needs background, which declares the permission it describes"],
    });
  }
});

void test("writes the declarations into the output directory tok gives", () => {
  const output = mkdtempSync(join(tmpdir(), "location-build-"));
  const script = join(dirname(fileURLToPath(import.meta.url)), "../build/index.mjs");
  const run = (platform: string, settings: Partial<Settings>) =>
    spawnSync(process.execPath, [script], {
      input: JSON.stringify({ platform, settings: { ...defaults, ...settings }, output }),
      encoding: "utf8",
    });

  const ios = run("ios", {});
  assert.equal(ios.status, 0, ios.stderr);
  assert.deepEqual(JSON.parse(ios.stdout), { plist: join(output, "Info.plist") });
  assert.ok(readFileSync(join(output, "Info.plist"), "utf8").includes(WHEN_IN_USE));

  const macos = run("macos", { precision: "approximate" });
  assert.equal(macos.status, 0);
  assert.match(macos.stderr, /^macOS has no declaration of approximate location/);

  const android = run("android", {});
  assert.deepEqual(JSON.parse(android.stdout), { manifest: join(output, "AndroidManifest.xml") });

  const rejected = run("ios", { "always-description": "Records your runs" });
  assert.equal(rejected.status, 1);
  assert.match(rejected.stderr, /always-description needs background/);
});
