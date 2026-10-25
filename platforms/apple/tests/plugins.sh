#!/usr/bin/env bash
# Typechecks each plugin's Swift with the Apple shell for macOS, iOS devices
# and the iOS Simulator at the pack's deployment targets, against the selected
# Xcode's SDKs: each plugin alone, then all of them in one module, as an app
# with every plugin compiles them. Each typechecks without compilation
# conditions, then with every condition its sources name.
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

typecheck_platforms() {
  typecheck "arm64-apple-macos$macos" macosx "$@"
  typecheck "arm64-apple-ios$ios" iphoneos "$@"
  typecheck "arm64-apple-ios$ios-simulator" iphonesimulator "$@"
}

macos=$(<"$root/platforms/apple/build/macos-deployment-target")
ios=$(<"$root/platforms/apple/build/ios-deployment-target")
# Swift requires unique file names in a module, so a plugin's source <name>
# is copied as <plugin>-<name>, as the pack's build does.
all=$temporary/all
mkdir "$all"
all_conditions=
for plugin in "$root"/plugins/*/apple; do
  sources=("$plugin"/*.swift)
  id=$(basename "$(dirname "$plugin")")
  echo "${plugin#"$root"/}"
  typecheck_platforms "${sources[@]}"
  # The pack's build accepts only conditions that start with the plugin's id
  # in capitals and contain only letters, digits and underscores.
  prefix=$(printf '%s' "$id" | tr 'a-z-' 'A-Z_')_
  conditions=$(grep -ohwE "${prefix}[A-Za-z0-9_]+" "${sources[@]}" | sort -u | sed 's/^/-D/' || true)
  if [[ -n "$conditions" ]]; then
    echo "${plugin#"$root"/} with its conditions"
    # shellcheck disable=SC2086 # Each condition is one word.
    typecheck_platforms $conditions "${sources[@]}"
    all_conditions+=" $conditions"
  fi
  for source in "${sources[@]}"; do cp "$source" "$all/$id-${source##*/}"; done
done
echo "all plugins"
typecheck_platforms "$all"/*.swift
if [[ -n "$all_conditions" ]]; then
  echo "all plugins with their conditions"
  # shellcheck disable=SC2086 # Each condition is one word.
  typecheck_platforms $all_conditions "$all"/*.swift
fi
