#!/usr/bin/env bash
# Describes the fixture plugin with the Android pack's entrypoint, as
# `tok plugin generate android` does, and checks the description against the
# platform API fixture, the dispatch against the committed one, and the
# dispatch's replies to the Worker's requests in a JVM unit test. Then checks
# that a new plugin's and every first-party plugin's Android files are what the
# pack generates.
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)
android=$root/platforms/android
fixture=$android/tests/fixture
temporary=$(mktemp -d)
trap 'rm -rf "$temporary"' EXIT

packs=$temporary/packs
pack=$packs/android-arm64
mkdir -p "$pack/build" "$pack/native-shell" "$pack/describer"
cp "$android/build/entrypoint" "$android/build/min-sdk" "$pack/build/"
cp -R "$android/source/." "$pack/native-shell/"
cp "$android/describer/"*.kt "$pack/describer/"

output=$temporary/output
mkdir "$output"
(cd "$pack" && bash build/entrypoint describe "$fixture" "$output" android)

# Android has one platform, so no method or event names the platforms that have it.
node -e '
  const { readFileSync } = require("node:fs");
  const [actual, expected] = process.argv.slice(1).map(path => JSON.parse(readFileSync(path, "utf8")));
  for (const method of [...expected.methods, ...expected.events]) delete method.platforms;
  require("node:assert/strict").deepEqual(actual, expected);
' "$output/description.json" "$root/plugins/core/tests/fixtures/description.json"
diff -u "$fixture/android/Exports.generated.kt" "$output/Exports.generated.kt"

project=$temporary/dispatch
mkdir -p "$project/plugin/src/main/kotlin" "$project/plugin/src/test/kotlin"
cp "$android/source/plugin/"*.kt "$fixture/android/"*.kt "$project/plugin/src/main/kotlin/"
cp "$android/tests/dispatch.kt" "$project/plugin/src/test/kotlin/"
cat > "$project/settings.gradle" <<'EOF'
pluginManagement {
  repositories {
    google()
    mavenCentral()
    gradlePluginPortal()
  }
}
dependencyResolutionManagement {
  repositories {
    google()
    mavenCentral()
  }
}
include ':plugin'
EOF
cat > "$project/build.gradle" <<'EOF'
plugins {
  id 'com.android.library' version '8.7.3' apply false
  id 'org.jetbrains.kotlin.android' version '2.1.0' apply false
}
EOF
cat > "$project/plugin/build.gradle" <<EOF
plugins {
  id 'com.android.library'
  id 'org.jetbrains.kotlin.android'
}

android {
  namespace 'tokamak.dispatch'
  compileSdk 35
  defaultConfig {
    minSdk $(<"$android/build/min-sdk")
  }
  compileOptions {
    sourceCompatibility JavaVersion.VERSION_17
    targetCompatibility JavaVersion.VERSION_17
  }
  testOptions {
    unitTests.all {
      testLogging {
        exceptionFormat 'full'
      }
    }
  }
}

kotlin {
  jvmToolchain(17)
  compilerOptions {
    freeCompilerArgs.add('-Xjvm-default=all')
    allWarningsAsErrors = true
  }
}

dependencies {
  implementation 'org.jetbrains.kotlinx:kotlinx-coroutines-android:1.9.0'
  testImplementation 'junit:junit:4.13.2'
  // android.jar's org.json is stubs; the JVM runs the reference implementation.
  testImplementation 'org.json:json:20250517'
}
EOF
gradle --quiet --project-dir "$project" :plugin:testDebugUnitTest

cargo build --quiet --manifest-path "$root/Cargo.toml" -p tokamak-cli
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$root/Cargo.toml")
printf '{"tokamakVersion": "%s", "target": "android-arm64", "variables": {}, "pluginKeys": {}}\n' "$version" \
  > "$pack/platform-pack.json"
# A new plugin starts with the files the pack generates, as do first-party ones, and its
# TypeScript composes each family's.
"$root/target/debug/tok" plugin new "$temporary/device-tools" --platforms ios,macos,android > /dev/null
mkdir -p "$temporary/device-tools/node_modules/@tokamakdev"
ln -s "$root/plugins/core" "$temporary/device-tools/node_modules/@tokamakdev/plugin"
"$root/plugins/node_modules/.bin/tsc" --project "$temporary/device-tools"
for registration in "$temporary/device-tools/android/Exports.kt" "$root"/plugins/*/android/Exports.kt; do
  checked=$(cd "${registration%/android/Exports.kt}" && TOKAMAK_PLATFORM_PACK_PATH=$packs "$root/target/debug/tok" plugin check 2>&1) ||
    { echo "$checked" >&2; exit 1; }
  # The stand-in pack describes android plugins, so they are checked, not skipped.
  if [[ $checked == *"android files are not checked"* ]]; then
    echo "$checked" >&2
    exit 1
  fi
done
