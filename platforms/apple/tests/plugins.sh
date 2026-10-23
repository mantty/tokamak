#!/usr/bin/env bash
# Typechecks each plugin's Swift with the Apple shell for macOS and iOS at the
# pack's deployment targets, against the selected Xcode's SDKs: each plugin
# alone, then all of them in one module, as an app with every plugin compiles
# them.
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)
temporary=$(mktemp -d)
trap 'rm -rf "$temporary"' EXIT
framework=$temporary/TokamakRuntime.framework
mkdir -p "$framework/Headers" "$framework/Modules"
cp "$root/platforms/apple/source/TokamakRuntime.h" "$framework/Headers/"
cp "$root/platforms/apple/source/module.modulemap" "$framework/Modules/"
printf 'func tokamakPlugins(app: TokamakApp) -> [any TokamakPlugin] { [] }\n' > "$temporary/registry.swift"

typecheck() {
  local target=$1 sdk=$2
  shift 2
  xcrun swiftc -typecheck -swift-version 5 -target "$target" \
    -sdk "$(xcrun --sdk "$sdk" --show-sdk-path)" \
    -F "$temporary" -module-cache-path "$temporary/cache" \
    "$temporary/registry.swift" "$root"/platforms/apple/source/*.swift "$@"
}

macos=$(<"$root/platforms/apple/build/macos-deployment-target")
ios=$(<"$root/platforms/apple/build/ios-deployment-target")
# Swift requires unique file names in a module, so a plugin's source <name>
# is copied as <plugin>-<name>, as the pack's build does.
all=$temporary/all
mkdir "$all"
for plugin in "$root"/plugins/*/apple; do
  sources=("$plugin"/*.swift)
  echo "${plugin#"$root"/}"
  typecheck "arm64-apple-macos$macos" macosx "${sources[@]}"
  typecheck "arm64-apple-ios$ios-simulator" iphonesimulator "${sources[@]}"
  id=$(basename "$(dirname "$plugin")")
  for source in "${sources[@]}"; do cp "$source" "$all/$id-${source##*/}"; done
done
echo "all plugins"
typecheck "arm64-apple-macos$macos" macosx "$all"/*.swift
typecheck "arm64-apple-ios$ios-simulator" iphonesimulator "$all"/*.swift
