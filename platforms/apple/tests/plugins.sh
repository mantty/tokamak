#!/usr/bin/env bash
# Typechecks each plugin's Swift with the Apple shell for macOS and iOS at the
# pack's deployment targets, against the selected Xcode's SDKs.
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
for plugin in "$root"/plugins/*/apple; do
  sources=("$plugin"/*.swift)
  echo "${plugin#"$root"/}"
  typecheck "arm64-apple-macos$macos" macosx "${sources[@]}"
  typecheck "arm64-apple-ios$ios-simulator" iphonesimulator "${sources[@]}"
done
