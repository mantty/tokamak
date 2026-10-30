#!/usr/bin/env bash
# Builds every first-party plugin's Swift package with the Apple shell and a
# registry that creates each plugin, as an app's build does, for macOS, iOS
# devices and the iOS Simulator at the pack's deployment targets, against the
# selected Xcode's SDKs: without compilation conditions, then with every
# condition the plugins' sources name. The runtime is its header alone, as the
# build compiles and does not link.
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)
apple=$root/platforms/apple
plugin_api=$root/plugins/core/apple/TokamakPlugin
temporary=$(mktemp -d)
trap 'rm -rf "$temporary"' EXIT

# The plugin API declares the pack's deployment targets, which plugins and
# apps build at.
ios=$(<"$apple/build/ios-deployment-target")
macos=$(<"$apple/build/macos-deployment-target")
if ! grep -qF "platforms: [.iOS(\"$ios\"), .macOS(\"$macos\")]" "$plugin_api/Package.swift"; then
  echo "plugins/core/apple/TokamakPlugin/Package.swift must declare iOS $ios and macOS $macos" >&2
  exit 1
fi

# Each plugin's package is staged as its id, beside the plugin API at the
# path its dependency on the API names.
packages=$temporary/packages
app=$temporary/app
mkdir -p "$packages/node_modules/@tokamakdev/plugin/apple" "$app/Sources/App" \
  "$app/Sources/TokamakRuntime/include"
staged_api=$packages/node_modules/@tokamakdev/plugin/apple/TokamakPlugin
ln -s "$plugin_api" "$staged_api"
cp "$apple/source/"*.swift "$app/Sources/App/"
cp "$apple/source/TokamakRuntime.h" "$app/Sources/TokamakRuntime/include/"
printf '' > "$app/Sources/TokamakRuntime/runtime.c"
dependencies=
products=
imports=
registrations=
conditions=
for plugin in "$root"/plugins/*/; do
  [[ -f "$plugin/tokamak-plugin.json" && -d "$plugin/apple" ]] || continue
  id=$(basename "$plugin")
  class=$(plutil -extract platforms.ios.class raw -o - "$plugin/tokamak-plugin.json")
  module=${class%%.*}
  rsync -a --exclude .build --exclude .swiftpm "$plugin/apple/" "$packages/$id/"
  dependencies+="    .package(path: \"$packages/$id\"),"$'\n'
  products+="        .product(name: \"$module\", package: \"$id\"),"$'\n'
  imports+="import $module"$'\n'
  registrations+="    $class(host: TokamakHost(app: app, plugin: \"$id\")),"$'\n'
  # The pack's build accepts only conditions that start with the plugin's id
  # in capitals and contain only letters, digits and underscores.
  prefix=$(printf '%s' "$id" | tr 'a-z-' 'A-Z_')_
  for condition in $(grep -rohwE "${prefix}[A-Za-z0-9_]+" "$plugin/apple" --include '*.swift' | sort -u); do
    conditions+=" $condition"
  done
done
{
  printf '%s\n' 'import TokamakPlugin'
  printf '%s' "$imports"
  printf '\n%s\n%s\n' 'func tokamakPlugins(app: TokamakApp) -> [any TokamakPlugin] {' '  ['
  printf '%s' "$registrations"
  printf '%s\n' '  ]' '}'
} > "$app/Sources/App/TokamakPluginRegistry.swift"
cat > "$app/Package.swift" <<EOF
// swift-tools-version: 5.9

import PackageDescription

let package = Package(
  name: "App",
  platforms: [.iOS("$ios"), .macOS("$macos")],
  products: [.library(name: "App", targets: ["App"])],
  dependencies: [
    .package(path: "$staged_api"),
$dependencies  ],
  targets: [
    .target(name: "TokamakRuntime"),
    .target(
      name: "App",
      dependencies: [
        "TokamakRuntime",
        .product(name: "TokamakPlugin", package: "TokamakPlugin"),
$products      ]
    ),
  ]
)
EOF

build() {
  local destination=$1 conditions=$2
  echo "$destination${conditions:+ with$conditions}"
  (
    cd "$app"
    xcodebuild build -quiet -scheme App -destination "$destination" \
      -derivedDataPath "$temporary/build" CODE_SIGNING_ALLOWED=NO \
      "SWIFT_ACTIVE_COMPILATION_CONDITIONS=\$(inherited)$conditions"
  )
}

for destination in 'generic/platform=macOS' 'generic/platform=iOS' 'generic/platform=iOS Simulator'; do
  build "$destination" ''
  build "$destination" "$conditions"
done
