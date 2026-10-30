#!/usr/bin/env bash
# The Android platform pack's entrypoint, run as the CLI runs it: from a pack
# root holding a stand-in runtime and shell, with fake Gradle and NDK compiler.
# With TOKAMAK_TEST_ANDROID_PACK set, it checks how that built pack links the
# runtime with the NDK at ANDROID_NDK_HOME instead.
set -euo pipefail

build=$(cd "$(dirname "${BASH_SOURCE[0]}")/../build" && pwd)
root=$(cd "$build/../../.." && pwd)
temporary=$(mktemp -d)
trap 'rm -rf "$temporary"' EXIT
tools=$temporary/tools
mkdir -p "$tools"

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

contains() {
  grep -qF -- "$2" "$1" || fail "$1 does not contain: $2"
}

lacks() {
  if grep -qF -- "$2" "$1"; then
    fail "$1 contains: $2"
  fi
}

# Run the remaining arguments, which must fail with the message $1.
fails_with() {
  local message=$1
  shift
  if "$@" 2> "$temporary/stderr"; then
    fail "succeeded: $*"
  fi
  contains "$temporary/stderr" "$message"
}

cat > "$tools/gradle" <<'EOF'
#!/bin/sh
set -eu
project=
next=
for argument in "$@"; do
  if [ "$next" = project ]; then
    project=$argument
  fi
  next=
  if [ "$argument" = --project-dir ]; then
    next=project
  fi
done
printf '%s\n' "$*" > "$project/gradle-arguments"
variant=debug
case " $* " in
  *" :app:assembleRelease "*) variant=release ;;
esac
mkdir -p "$project/app/build/outputs/apk/$variant"
touch "$project/app/build/outputs/apk/$variant/app-$variant.apk"
EOF
chmod +x "$tools/gradle"
export PATH="$tools:$PATH"

if [[ -z "${TOKAMAK_TEST_ANDROID_PACK:-}" ]]; then
  pack=$temporary/pack
  mkdir -p "$pack/build" "$pack/lib/TokamakRuntime" "$pack/native-shell/app" "$pack/plugin-api/src/main/kotlin"
  cp "$build/entrypoint" "$build/min-sdk" "$pack/build/"
  printf runtime > "$pack/lib/TokamakRuntime/libtokamak.a"
  printf %s -llog > "$pack/lib/TokamakRuntime/link-libraries"
  touch "$pack/native-shell/app/TokamakActivity.kt" "$pack/plugin-api/src/main/kotlin/TokamakPlugin.kt"
  printf %s 'plugins {}' > "$pack/plugin-api/build.gradle.kts"
  compiler=$temporary/ndk/toolchains/llvm/prebuilt/host/bin/aarch64-linux-android$(<"$build/min-sdk")-clang
  mkdir -p "$(dirname "$compiler")"
  cat > "$compiler" <<'EOF'
#!/bin/sh
while [ "$#" -gt 0 ]; do
  if [ "$1" = -o ]; then
    : > "$2"
  fi
  shift
done
EOF
  chmod +x "$compiler"
  export ANDROID_NDK_HOME=$temporary/ndk
else
  pack=$TOKAMAK_TEST_ANDROID_PACK
fi
unset TOKAMAK_ANDROID_KEYSTORE TOKAMAK_ANDROID_KEYSTORE_PASSWORD TOKAMAK_ANDROID_KEY_ALIAS \
  TOKAMAK_ANDROID_KEY_PASSWORD TOKAMAK_ANDROID_MANIFEST TOKAMAK_ANDROID_ICON

# Stage a build input for the case $1 in $case, whose app the entrypoint builds
# into $output and whose Gradle project is $gradle.
new_case() {
  case=$temporary/$1
  input=$case/input
  output=$case/build/demo-app.apk
  gradle=$case/build/.tokamak
  mkdir -p "$input/metadata" "$input/app" "$input/plugins"
  printf %s 'Demo App' > "$input/metadata/app-name"
  printf %s demo-app.tokamak.local > "$input/metadata/host"
  printf %s com.example.demo > "$input/metadata/identifier"
  printf %s android > "$input/metadata/platform"
  printf %s android-arm64 > "$input/metadata/target"
  printf %s 1.2.3 > "$input/metadata/version"
  : > "$input/metadata/exported-symbols"
  printf %s '<html></html>' > "$input/app/index.html"
}

# Run the entrypoint from the pack root, with the environment assignments given.
entrypoint() {
  (cd "$pack" && env "$@" bash build/entrypoint build "$input" "$output")
}

# Stage the plugin `alerts`, whose Android class is $1, or test.alerts.Plugin,
# and whose Gradle module is native.
stage_plugin() {
  local plugin=$input/plugins/alerts
  mkdir -p "$plugin/native/src/main/kotlin/rules"
  printf %s "${1:-test.alerts.Plugin}" > "$plugin/class"
  printf %s "implementation(\"com.example:messaging:1.2.3\")" > "$plugin/native/build.gradle.kts"
  : > "$plugin/native/src/main/kotlin/Plugin.kt"
  : > "$plugin/native/src/main/kotlin/rules/Rules.kt"
}

test_builds_shrunk_lint_checked_release_apps() {
  new_case release
  printf %s "1.0'beta\"" > "$input/metadata/version"
  printf %s "Vigilus & <Co> \"Pro\" 'X'" > "$input/metadata/app-name"
  entrypoint
  [[ -f "$output" ]] || fail "no APK at $output"
  contains "$gradle/gradle-arguments" "-Ptokamak.version=1.0'beta\" :app:lintRelease :app:assembleRelease"
  local script=$gradle/app/build.gradle
  contains "$script" "versionName providers.gradleProperty('tokamak.version').get()"
  contains "$script" "minSdk $(<"$build/min-sdk")"
  contains "$script" "checkOnly 'NewApi'"
  contains "$script" "abortOnError true"
  contains "$script" "checkDependencies true"
  contains "$gradle/build.gradle" "freeCompilerArgs.add('-Xjvm-default=all')"
  contains "$gradle/gradle.properties" "org.gradle.jvmargs=-Xmx4g"
  contains "$script" "minifyEnabled true"
  contains "$script" "signingConfig signingConfigs.debug"
  lacks "$script" "signingConfigs {"
  contains "$gradle/app/tokamak-rules.pro" "-dontobfuscate"
  contains "$gradle/app/tokamak-rules.pro" '-keep interface com.tokamak.runtime.TokamakRuntime$Plugins { *; }'
  contains "$gradle/app/src/main/AndroidManifest.xml" 'android:label="Vigilus &amp; &lt;Co&gt; &quot;Pro&quot; &apos;X&apos;"'
  contains "$gradle/tokamak-exports.map" "    Java_*;"
  lacks "$gradle/tokamak-exports.map" tokamak_storage
  [[ -f "$gradle/app/src/main/jniLibs/arm64-v8a/libtokamak.so" ]] || fail "the runtime was not linked"
}

test_exports_the_runtime_parts_the_app_links() {
  new_case storage
  printf '%s\n' tokamak_storage > "$input/metadata/exported-symbols"
  entrypoint
  contains "$gradle/tokamak-exports.map" "    tokamak_storage;"
}

test_signs_release_builds_with_the_configured_keystore() {
  new_case keystore
  local keystore=$case/release.keystore
  printf keystore > "$keystore"
  local missing="an Android keystore needs key-alias and keystore-password"
  fails_with "$missing" entrypoint TOKAMAK_ANDROID_KEYSTORE="$keystore" TOKAMAK_ANDROID_KEYSTORE_PASSWORD=secret
  fails_with "$missing" entrypoint TOKAMAK_ANDROID_KEYSTORE="$keystore" TOKAMAK_ANDROID_KEY_ALIAS=release
  fails_with "Android keystore is missing" entrypoint TOKAMAK_ANDROID_KEYSTORE="$case/missing.keystore" \
    TOKAMAK_ANDROID_KEYSTORE_PASSWORD=secret TOKAMAK_ANDROID_KEY_ALIAS=release
  fails_with "Android key-alias and passwords need a keystore" entrypoint TOKAMAK_ANDROID_KEY_ALIAS=release
  entrypoint TOKAMAK_ANDROID_KEYSTORE="$keystore" TOKAMAK_ANDROID_KEYSTORE_PASSWORD=secret TOKAMAK_ANDROID_KEY_ALIAS=release
  local script=$gradle/app/build.gradle
  contains "$script" "signingConfig signingConfigs.release"
  contains "$script" "storeFile file(System.getenv('TOKAMAK_ANDROID_KEYSTORE'))"
  contains "$script" "keyPassword System.getenv('TOKAMAK_ANDROID_KEY_PASSWORD') ?: System.getenv('TOKAMAK_ANDROID_KEYSTORE_PASSWORD')"
  lacks "$script" secret
}

test_builds_development_apps_as_debug_builds() {
  new_case development
  printf %s http://127.0.0.1:9 > "$input/metadata/dev-endpoint"
  printf %s token > "$input/metadata/dev-session-token"
  # Development builds ignore release signing settings.
  entrypoint TOKAMAK_ANDROID_KEYSTORE="$case/missing.keystore"
  contains "$gradle/gradle-arguments" ":app:lintDebug :app:assembleDebug"
  contains "$gradle/app/build.gradle" "signingConfigs.debug"
  contains "$gradle/app/src/main/AndroidManifest.xml" '<meta-data android:name="tokamak.dev.endpoint" android:value="http://127.0.0.1:9" />'
  contains "$gradle/app/src/main/AndroidManifest.xml" '<meta-data android:name="tokamak.dev.session-token" android:value="token" />'
  [[ -f "$output" ]] || fail "no APK at $output"
}

test_merges_the_app_manifest_while_it_is_set() {
  new_case manifest
  local manifest=$case/AndroidManifest.xml
  printf %s '<manifest><uses-permission android:name="android.permission.CAMERA" /></manifest>' > "$manifest"
  fails_with "Android manifest file is missing" entrypoint TOKAMAK_ANDROID_MANIFEST="$case/missing.xml"
  entrypoint TOKAMAK_ANDROID_MANIFEST="$manifest"
  cmp -s "$manifest" "$gradle/app/user/AndroidManifest.xml" || fail "the app manifest was not staged"
  contains "$gradle/app/build.gradle" "addStaticManifestFile(file('user/AndroidManifest.xml').path)"
  entrypoint
  [[ ! -e "$gradle/app/user" ]] || fail "the app manifest outlived its setting"
  lacks "$gradle/app/build.gradle" addStaticManifestFile
}

test_uses_the_icon_resources() {
  new_case icon
  local icon=$case/icon
  mkdir -p "$icon/mipmap-mdpi"
  fails_with "Android icon resources must define mipmap/ic_launcher" entrypoint TOKAMAK_ANDROID_ICON="$icon"
  printf png > "$icon/mipmap-mdpi/ic_launcher.png"
  fails_with "android.icon must be an Android res directory: $icon/mipmap-mdpi/ic_launcher.png" \
    entrypoint TOKAMAK_ANDROID_ICON="$icon/mipmap-mdpi/ic_launcher.png"
  entrypoint TOKAMAK_ANDROID_ICON="$icon"
  [[ $(<"$gradle/app/src/main/res/mipmap-mdpi/ic_launcher.png") == png ]] || fail "the icon was not staged"
  contains "$gradle/app/src/main/AndroidManifest.xml" 'android:icon="@mipmap/ic_launcher"'
}

test_builds_each_plugin_s_gradle_module_into_the_app() {
  new_case plugins
  stage_plugin
  entrypoint
  diff -r "$input/plugins/alerts/native" "$gradle/plugins/alerts" || fail "the plugin module was not staged as it is"
  diff -r "$pack/plugin-api" "$gradle/tokamak-plugin" || fail "the plugin API module was not staged as it is"
  contains "$gradle/settings.gradle" "include ':plugins:alerts'"
  contains "$gradle/app/build.gradle" "implementation project(':plugins:alerts')"
  contains "$gradle/app/src/main/kotlin/com/tokamak/runtime/TokamakPluginRegistry.kt" 'test.alerts.Plugin(app.host("alerts")),'
  contains "$gradle/app/src/main/AndroidManifest.xml" '<uses-permission android:name="android.permission.INTERNET" />'
  [[ ! -e "$gradle/generated/alerts" ]] || fail "a plugin without generated files has a generated module"
  lacks "$gradle/settings.gradle" ":generated:alerts"
}

test_builds_the_files_plugins_build_scripts_generate_as_library_modules() {
  new_case generated
  stage_plugin
  local plugin=$input/plugins/alerts
  printf %s '<manifest><uses-permission android:name="android.permission.USE_BIOMETRIC" /></manifest>' > "$plugin/manifest"
  mkdir -p "$plugin/raw"
  printf wav > "$plugin/raw/0-chime.wav"
  entrypoint
  local module=$gradle/generated/alerts
  cmp -s "$plugin/manifest" "$module/src/main/AndroidManifest.xml" || fail "the generated manifest was not staged"
  [[ $(<"$module/src/main/res/raw/chime.wav") == wav ]] || fail "the raw resource was not staged by its name"
  contains "$module/build.gradle" "namespace 'tokamak.generated.alerts'"
  contains "$module/build.gradle" "minSdk $(<"$build/min-sdk")"
  contains "$gradle/settings.gradle" "include ':generated:alerts'"
  contains "$gradle/app/build.gradle" "implementation project(':generated:alerts')"
  lacks "$gradle/app/src/main/AndroidManifest.xml" USE_BIOMETRIC
}

test_creates_the_dispatch_of_plugins_that_export_methods() {
  new_case exports
  stage_plugin
  : > "$input/plugins/alerts/native/src/main/kotlin/rules/Exports.kt"
  entrypoint
  contains "$gradle/app/src/main/kotlin/com/tokamak/runtime/TokamakPluginRegistry.kt" 'test.alerts.PluginDispatch(app.host("alerts", test.alerts.PluginDispatch)),'
}

test_rejects_invalid_plugin_classes_and_plugins_without_gradle_modules() {
  new_case invalid-class
  stage_plugin Plugin
  fails_with "plugin 'alerts' has invalid android class 'Plugin'" entrypoint
  new_case no-module
  stage_plugin
  rm "$input/plugins/alerts/native/build.gradle.kts"
  fails_with "plugin 'alerts' has no Gradle module, android/build.gradle.kts" entrypoint
}

test_writes_the_lock_without_one_and_otherwise_builds_as_it_pins() {
  new_case lock
  entrypoint
  contains "$gradle/gradle-arguments" "--write-locks tokamakLockDependencies"
  contains "$gradle/build.gradle" "lockAllConfigurations()"
  contains "$gradle/build.gradle" "lockMode = LockMode.LENIENT"
  contains "$gradle/build.gradle" "lockFile = file('$input/lock/' + path.substring(1).replace(':', '.') + '.lockfile')"
  rm -rf "${input:?}/lock"
  mkdir "$input/lock"
  : > "$input/lock/app.lockfile"
  entrypoint
  lacks "$gradle/gradle-arguments" "--write-locks"
  lacks "$gradle/gradle-arguments" tokamakLockDependencies
}

test_the_plugin_api_supports_the_pack_s_minimum_sdk() {
  contains "$root/plugins/core/android/build.gradle.kts" "minSdk = $(<"$build/min-sdk")"
}

# Whether the runtime library of the last build exports the entry point $1 and
# contains $2, a string only that part's code holds.
linked() {
  local library=$gradle/app/src/main/jniLibs/arm64-v8a/libtokamak.so
  local symbols
  symbols=$("$ANDROID_NDK_HOME"/toolchains/llvm/prebuilt/*/bin/llvm-nm --dynamic --defined-only "$library")
  [[ $symbols == *" Java_"* ]] || fail "$library exports no JNI functions"
  echo "$library: $(wc -c < "$library") bytes" >&2
  printf '%s %s\n' \
    "$([[ $symbols == *"$1"* ]] && echo exported || echo absent)" \
    "$(grep -qF -- "$2" "$library" && echo linked || echo omitted)"
}

test_links_storage_from_the_platform_pack_while_the_app_declares_it() {
  new_case linked-storage
  entrypoint
  # SQLite writes this header into every database.
  [[ $(linked tokamak_storage 'SQLite format 3') == "absent omitted" ]] || fail "storage is linked without a storage binding"
  printf '%s\n' tokamak_storage > "$input/metadata/exported-symbols"
  entrypoint
  [[ $(linked tokamak_storage 'SQLite format 3') == "exported linked" ]] || fail "storage is not linked with a storage binding"
}

test_links_development_from_the_platform_pack_while_the_app_declares_it() {
  new_case linked-development
  entrypoint
  # The dev socket marks its requests with this header.
  [[ $(linked tokamak_development x-tokamak-dev-socket) == "absent omitted" ]] || fail "development is linked into a release app"
  printf '%s\n' tokamak_development > "$input/metadata/exported-symbols"
  entrypoint
  [[ $(linked tokamak_development x-tokamak-dev-socket) == "exported linked" ]] || fail "development is not linked while the app declares it"
}

if [[ -n "${TOKAMAK_TEST_ANDROID_PACK:-}" ]]; then
  tests=(
    test_links_storage_from_the_platform_pack_while_the_app_declares_it
    test_links_development_from_the_platform_pack_while_the_app_declares_it
  )
else
  tests=(
    test_builds_shrunk_lint_checked_release_apps
    test_exports_the_runtime_parts_the_app_links
    test_signs_release_builds_with_the_configured_keystore
    test_builds_development_apps_as_debug_builds
    test_merges_the_app_manifest_while_it_is_set
    test_uses_the_icon_resources
    test_builds_each_plugin_s_gradle_module_into_the_app
    test_builds_the_files_plugins_build_scripts_generate_as_library_modules
    test_creates_the_dispatch_of_plugins_that_export_methods
    test_rejects_invalid_plugin_classes_and_plugins_without_gradle_modules
    test_writes_the_lock_without_one_and_otherwise_builds_as_it_pins
    test_the_plugin_api_supports_the_pack_s_minimum_sdk
  )
fi
for test in "${tests[@]}"; do
  "$test" > /dev/null
  echo "ok ${test#test_}"
done
