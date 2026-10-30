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
sources=$fixture/android/src/main/kotlin
plugin_api=$root/plugins/core/android
temporary=$(mktemp -d)
trap 'rm -rf "$temporary"' EXIT

packs=$temporary/packs
pack=$packs/android-arm64
mkdir -p "$pack/build" "$pack/native-shell" "$pack/describer" "$pack/plugin-api"
cp "$android/build/entrypoint" "$android/build/min-sdk" "$pack/build/"
cp -R "$android/source/." "$pack/native-shell/"
cp "$android/describer/"*.kt "$pack/describer/"
cp -R "$plugin_api/build.gradle.kts" "$plugin_api/src" "$pack/plugin-api/"

output=$temporary/output
mkdir "$output"
(cd "$pack" && bash build/entrypoint describe "$fixture" "$sources/Exports.kt" "$output" android)

# Android has one platform, so no method or event names the platforms that have it.
node -e '
  const { readFileSync } = require("node:fs");
  const [actual, expected] = process.argv.slice(1).map(path => JSON.parse(readFileSync(path, "utf8")));
  for (const method of [...expected.methods, ...expected.events]) delete method.platforms;
  require("node:assert/strict").deepEqual(actual, expected);
' "$output/description.json" "$root/plugins/core/tests/fixtures/description.json"
diff -u "$sources/Exports.generated.kt" "$output/Exports.generated.kt"

# The dispatch test depends on the fixture's module and the plugin API's, which
# build outside their directories with the pack's build tools, without
# warnings.
agp_version=$(sed -n 's/^agp_version=//p' "$android/build/entrypoint")
kotlin_version=$(sed -n 's/^kotlin_version=//p' "$android/build/entrypoint")
project=$temporary/dispatch
mkdir -p "$project/dispatch/src/test/kotlin"
cp "$android/tests/dispatch.kt" "$project/dispatch/src/test/kotlin/"
cat > "$project/settings.gradle" <<EOF
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
include ':tokamak-plugin', ':plugin', ':dispatch'
project(':tokamak-plugin').projectDir = file('$plugin_api')
project(':plugin').projectDir = file('$fixture/android')
EOF
cat > "$project/build.gradle" <<EOF
plugins {
  id 'com.android.library' version '$agp_version' apply false
  id 'org.jetbrains.kotlin.android' version '$kotlin_version' apply false
}

subprojects {
  layout.buildDirectory.set(rootProject.layout.buildDirectory.dir(name))
  plugins.withId('org.jetbrains.kotlin.android') {
    kotlin {
      jvmToolchain(17)
      compilerOptions {
        freeCompilerArgs.add('-Xjvm-default=all')
        allWarningsAsErrors = true
      }
    }
  }
}
EOF
printf '%s\n' 'android.useAndroidX=true' > "$project/gradle.properties"
cat > "$project/dispatch/build.gradle" <<EOF
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
  testOptions {
    unitTests.all {
      testLogging {
        exceptionFormat 'full'
      }
    }
  }
}

dependencies {
  testImplementation project(':tokamak-plugin')
  testImplementation project(':plugin')
  testImplementation 'junit:junit:4.13.2'
  // android.jar's org.json is stubs; the JVM runs the reference implementation.
  testImplementation 'org.json:json:20250517'
}
EOF
gradle --quiet --project-dir "$project" :dispatch:testDebugUnitTest

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
for plugin in "$temporary/device-tools" "$root"/plugins/*/; do
  [[ -f "$plugin/tokamak-plugin.json" && -d "$plugin/android" ]] || continue
  checked=$(cd "$plugin" && TOKAMAK_PLATFORM_PACK_PATH=$packs "$root/target/debug/tok" plugin check 2>&1) ||
    { echo "$checked" >&2; exit 1; }
  # The stand-in pack describes android plugins, so they are checked, not skipped.
  if [[ $checked == *"android files are not checked"* ]]; then
    echo "$checked" >&2
    exit 1
  fi
done
