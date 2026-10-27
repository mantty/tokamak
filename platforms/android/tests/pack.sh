#!/usr/bin/env bash
# The Android platform pack's entrypoint, run as the CLI runs it: from a pack
# root holding a stand-in runtime and shell, with fake Gradle and NDK compiler.
# With TOKAMAK_TEST_ANDROID_PACK set, it checks how that built pack links the
# runtime with the NDK at ANDROID_NDK_HOME instead.
set -euo pipefail

build=$(cd "$(dirname "${BASH_SOURCE[0]}")/../build" && pwd)
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
  mkdir -p "$pack/build" "$pack/lib/TokamakRuntime" "$pack/native-shell/app" "$pack/native-shell/plugin"
  cp "$build/entrypoint" "$build/min-sdk" "$pack/build/"
  printf runtime > "$pack/lib/TokamakRuntime/libtokamak.a"
  printf %s -llog > "$pack/lib/TokamakRuntime/link-libraries"
  touch "$pack/native-shell/app/TokamakActivity.kt" "$pack/native-shell/plugin/TokamakPlugin.kt"
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
  # Builds create no gateway certificate unless a case gives them the JDK.
  jdk=${JAVA_HOME:-}
  mkdir -p "$temporary/jdk/bin"
  printf '#!/bin/sh\n' > "$temporary/jdk/bin/keytool"
  chmod +x "$temporary/jdk/bin/keytool"
  export JAVA_HOME=$temporary/jdk
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

# Stage the plugin `alerts`, whose Android section is test.alerts.Plugin.
stage_plugin() {
  local plugin=$input/plugins/alerts
  mkdir -p "$plugin/native/rules" "$plugin/dependencies"
  printf %s "${1:-test.alerts.Plugin}" > "$plugin/class"
  : > "$plugin/native/Plugin.kt"
  : > "$plugin/native/rules/Rules.kt"
  : > "$plugin/native/rules/Plugin.kt"
  : > "$plugin/native/Info.plist"
  printf %s "${2:-com.example:messaging:1.2.3}" > "$plugin/dependencies/0"
  printf %s '<manifest><uses-permission android:name="android.permission.USE_BIOMETRIC" /></manifest>' > "$plugin/manifest"
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
  contains "$gradle/build.gradle" "minSdk $(<"$build/min-sdk")"
  contains "$script" "checkOnly 'NewApi'"
  contains "$script" "abortOnError true"
  contains "$script" "checkDependencies true"
  contains "$gradle/build.gradle" "freeCompilerArgs.add('-Xjvm-default=all')"
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

test_trusts_the_gateway_certificate_each_build_creates() {
  new_case gateway
  entrypoint JAVA_HOME="$jdk"
  local resources=$gradle/app/src/main/res
  local config=$resources/xml/tokamak_network_security_config.xml
  local manifest=$gradle/app/src/main/AndroidManifest.xml
  contains "$manifest" 'android:networkSecurityConfig="@xml/tokamak_network_security_config"'
  lacks "$manifest" usesCleartextTraffic
  contains "$config" '<base-config cleartextTrafficPermitted="false" />'
  contains "$config" '<domain includeSubdomains="false">demo-app.tokamak.local</domain>'
  contains "$config" '<certificates src="@raw/tokamak_gateway_authority" />'
  local keytool=${jdk:+$jdk/bin/}keytool
  local p12=$gradle/app/src/main/assets/tokamak-gateway.p12
  "$keytool" -list -v -keystore "$p12" -storepass tokamak > "$case/keystore"
  "$keytool" -printcert -file "$resources/raw/tokamak_gateway_authority.pem" > "$case/authority"
  # The keystore holds the gateway's key and chain, not the authority's key.
  contains "$case/keystore" "Your keystore contains 1 entry"
  contains "$case/keystore" "Alias name: gateway"
  contains "$case/keystore" "Certificate chain length: 2"
  contains "$case/keystore" "Owner: CN=demo-app.tokamak.local"
  contains "$case/keystore" "DNSName: demo-app.tokamak.local"
  contains "$case/keystore" "serverAuth"
  contains "$case/keystore" "DigitalSignature"
  contains "$case/authority" "Owner: CN=tokamak gateway"
  contains "$case/authority" "CA:true"
  contains "$case/authority" "PathLen:0"
  # Installed apps trust both certificates for as long as they stay installed.
  local year
  for year in $(grep "until:" "$case/keystore" | awk '{print $NF}'); do
    ((year >= $(date +%Y) + 99)) || fail "a gateway certificate expires in $year"
  done
  # Older Android releases read only PKCS#12 files in the classic format.
  openssl pkcs12 -in "$p12" -info -noout -passin pass:tokamak > "$case/pkcs12" 2>&1
  contains "$case/pkcs12" "Shrouded Keybag: pbeWithSHA1And3-KeyTripleDES-CBC, Iteration 1"
  lacks "$case/pkcs12" "Encrypted data"
  local authority
  authority=$(grep -m1 "SHA256:" "$case/authority")
  contains "$case/keystore" "$authority"
  entrypoint JAVA_HOME="$jdk"
  "$keytool" -printcert -file "$resources/raw/tokamak_gateway_authority.pem" > "$case/rebuilt"
  lacks "$case/rebuilt" "$authority"
  fails_with "Android builds create the gateway certificate with keytool" entrypoint JAVA_HOME="$case/no-jdk"
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

test_builds_each_plugin_as_a_library_module() {
  new_case plugins
  stage_plugin
  entrypoint
  local module=$gradle/plugins/alerts
  cmp -s "$input/plugins/alerts/manifest" "$module/src/main/AndroidManifest.xml" || fail "the plugin manifest was not staged"
  local sources=$module/src/main/kotlin
  [[ -f "$sources/Plugin.kt" && -f "$sources/rules/Rules.kt" && -f "$sources/rules/Plugin.kt" ]] || fail "the plugin sources were not staged by path"
  [[ ! -e "$module/src/main/kotlin/Info.plist" ]] || fail "a file that is not Kotlin was staged as a source"
  local coroutines
  coroutines=$(sed -n 's/^coroutines_version=//p' "$build/entrypoint")
  contains "$gradle/tokamak-plugin/build.gradle" "api 'org.jetbrains.kotlinx:kotlinx-coroutines-android:$coroutines'"
  contains "$module/build.gradle" "implementation project(':tokamak-plugin')"
  contains "$module/build.gradle" "implementation 'com.example:messaging:1.2.3'"
  contains "$gradle/settings.gradle" "include ':plugins:alerts'"
  contains "$gradle/app/build.gradle" "implementation project(':plugins:alerts')"
  contains "$gradle/app/src/main/kotlin/com/tokamak/runtime/TokamakPluginRegistry.kt" 'test.alerts.Plugin(app.host("alerts")),'
  contains "$gradle/app/src/main/AndroidManifest.xml" '<uses-permission android:name="android.permission.INTERNET" />'
  lacks "$gradle/app/src/main/AndroidManifest.xml" USE_BIOMETRIC
}

test_creates_the_dispatch_of_plugins_that_export_methods() {
  new_case exports
  stage_plugin
  : > "$input/plugins/alerts/native/Exports.kt"
  entrypoint
  contains "$gradle/app/src/main/kotlin/com/tokamak/runtime/TokamakPluginRegistry.kt" 'test.alerts.PluginDispatch(app.host("alerts", test.alerts.PluginDispatch)),'
}

test_rejects_invalid_plugin_classes_and_dependencies() {
  new_case invalid-class
  stage_plugin Plugin
  fails_with "plugin 'alerts' has invalid android class 'Plugin'" entrypoint
  new_case invalid-dependency
  stage_plugin test.alerts.Plugin com.example:library
  fails_with "plugin 'alerts' has invalid android dependency 'com.example:library'; use group:artifact:version" entrypoint
  new_case quoted-dependency
  stage_plugin test.alerts.Plugin "com.example:library:1.0'"
  fails_with "plugin 'alerts' has invalid android dependency 'com.example:library:1.0''" entrypoint
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
    test_trusts_the_gateway_certificate_each_build_creates
    test_exports_the_runtime_parts_the_app_links
    test_signs_release_builds_with_the_configured_keystore
    test_builds_development_apps_as_debug_builds
    test_merges_the_app_manifest_while_it_is_set
    test_uses_the_icon_resources
    test_builds_each_plugin_as_a_library_module
    test_creates_the_dispatch_of_plugins_that_export_methods
    test_rejects_invalid_plugin_classes_and_dependencies
  )
fi
for test in "${tests[@]}"; do
  "$test" > /dev/null
  echo "ok ${test#test_}"
done
