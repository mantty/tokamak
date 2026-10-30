#!/usr/bin/env bash
# Describes the fixture plugin with the Apple pack's entrypoint, as
# `tok plugin generate apple` does, and checks the description against the
# platform API fixture, the dispatch against the committed one, and the
# dispatch's replies to the Worker's requests. Then checks that a new plugin's
# and every first-party plugin's Apple files are what the pack generates.
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)
apple=$root/platforms/apple
fixture=$apple/tests/fixture
sources=$fixture/apple/Sources/TokamakExample
temporary=$(mktemp -d)
trap 'rm -rf "$temporary"' EXIT

# A debug build, which `swift test` shares.
describer=(swift build --package-path "$apple/describer")
"${describer[@]}"

packs=$temporary/packs
target=macos-$(uname -m | sed 's/x86_64/x64/')
pack=$packs/$target
mkdir -p "$pack/build" "$pack/plugin-api/TokamakPlugin" "$pack/tools"
cp "$apple/build/entrypoint" "$apple/build/"*-deployment-target "$pack/build/"
cp -R "$root/plugins/core/apple/TokamakPlugin/Package.swift" "$root/plugins/core/apple/TokamakPlugin/Sources" \
  "$pack/plugin-api/TokamakPlugin/"
cp "$("${describer[@]}" --show-bin-path)/tokamak-swift-describe" "$pack/tools/"
printf '#!/bin/sh\n' > "$pack/tools/tokamak-apple-signing"
chmod +x "$pack/tools/tokamak-apple-signing"

output=$temporary/output
mkdir "$output"
(cd "$pack" && bash build/entrypoint describe "$fixture" "$sources/Exports.swift" "$output" ios macos)

node -e '
  const { readFileSync } = require("node:fs");
  const [actual, expected] = process.argv.slice(1).map(path => JSON.parse(readFileSync(path, "utf8")));
  require("node:assert/strict").deepEqual(actual, expected);
' "$output/description.json" "$root/plugins/core/tests/fixtures/description.json"
diff -u "$sources/Exports.generated.swift" "$output/Exports.generated.swift"
xcrun swift-format lint --strict "$fixture/apple/Package.swift" "$sources/"*.swift

# The dispatch tests run in a package that depends on the fixture's, staged
# beside the plugin API at the path its dependency names.
packages=$temporary/packages
mkdir -p "$packages/node_modules/@tokamakdev/plugin/apple" "$packages/dispatch/Sources/DispatchTests"
ln -s "$pack/plugin-api/TokamakPlugin" "$packages/node_modules/@tokamakdev/plugin/apple/TokamakPlugin"
cp -R "$fixture/apple" "$packages/example"
cp "$apple/tests/dispatch.swift" "$packages/dispatch/Sources/DispatchTests/"
cat > "$packages/dispatch/Package.swift" <<EOF
// swift-tools-version: 5.9

import PackageDescription

let package = Package(
  name: "DispatchTests",
  platforms: [.macOS("14.0")],
  dependencies: [
    .package(path: "$packages/node_modules/@tokamakdev/plugin/apple/TokamakPlugin"),
    .package(path: "$packages/example"),
  ],
  targets: [
    .executableTarget(
      name: "DispatchTests",
      dependencies: [
        .product(name: "TokamakPlugin", package: "TokamakPlugin"),
        .product(name: "TokamakExample", package: "example"),
      ])
  ]
)
EOF
swift run --quiet --package-path "$packages/dispatch"

cargo build --quiet --manifest-path "$root/Cargo.toml" -p tokamak-cli
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$root/Cargo.toml")
printf '{"tokamakVersion": "%s", "target": "%s", "variables": {}, "pluginKeys": {}}\n' "$version" "$target" \
  > "$pack/platform-pack.json"
# A new plugin starts with the files the pack generates, as do first-party ones.
"$root/target/debug/tok" plugin new "$temporary/device-tools" --platforms ios,macos > /dev/null
for plugin in "$temporary/device-tools" "$root"/plugins/*/; do
  [[ -f "$plugin/tokamak-plugin.json" && -d "$plugin/apple" ]] || continue
  checked=$(cd "$plugin" && TOKAMAK_PLATFORM_PACK_PATH=$packs "$root/target/debug/tok" plugin check 2>&1) ||
    { echo "$checked" >&2; exit 1; }
  # The stand-in pack describes apple plugins, so they are checked, not skipped.
  if [[ $checked == *"apple files are not checked"* ]]; then
    echo "$checked" >&2
    exit 1
  fi
done
