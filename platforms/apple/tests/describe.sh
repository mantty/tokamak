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
temporary=$(mktemp -d)
trap 'rm -rf "$temporary"' EXIT

# A debug build, which `swift test` shares.
describer=(swift build --package-path "$apple/describer")
"${describer[@]}"

packs=$temporary/packs
target=macos-$(uname -m | sed 's/x86_64/x64/')
pack=$packs/$target
framework=$pack/frameworks/TokamakRuntime.framework
mkdir -p "$pack/build" "$pack/native-shell" "$pack/tools" "$framework/Headers" "$framework/Modules"
cp "$apple/build/entrypoint" "$apple/build/"*-deployment-target "$pack/build/"
cp "$apple/source/"*.swift "$pack/native-shell/"
cp "$apple/source/TokamakRuntime.h" "$framework/Headers/"
cp "$apple/source/module.modulemap" "$framework/Modules/"
cp "$("${describer[@]}" --show-bin-path)/tokamak-swift-describe" "$pack/tools/"
printf '#!/bin/sh\n' > "$pack/tools/tokamak-apple-signing"
chmod +x "$pack/tools/tokamak-apple-signing"

output=$temporary/output
mkdir "$output"
(cd "$pack" && bash build/entrypoint describe "$fixture" "$output" ios macos)

node -e '
  const { readFileSync } = require("node:fs");
  const [actual, expected] = process.argv.slice(1).map(path => JSON.parse(readFileSync(path, "utf8")));
  require("node:assert/strict").deepEqual(actual, expected);
' "$output/description.json" "$root/plugins/core/tests/fixtures/description.json"
diff -u "$fixture/apple/Exports.generated.swift" "$output/Exports.generated.swift"
xcrun swift-format lint --strict "$fixture/apple/"*.swift

xcrun swiftc -swift-version 5 -parse-as-library -F "$pack/frameworks" -module-cache-path "$temporary/cache" \
  "$apple/source/TokamakPlugins.swift" "$apple/source/TokamakOrigin.swift" \
  "$apple/source/TokamakResources.swift" "$apple/source/TokamakControls.swift" \
  "$apple/source/TokamakExports.swift" "$fixture/apple/"*.swift "$apple/tests/dispatch.swift" \
  -o "$temporary/dispatch"
"$temporary/dispatch"

cargo build --quiet --manifest-path "$root/Cargo.toml" -p tokamak-cli
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$root/Cargo.toml")
printf '{"tokamakVersion": "%s", "target": "%s", "variables": {}, "pluginKeys": {}}\n' "$version" "$target" \
  > "$pack/platform-pack.json"
# A new plugin starts with the files the pack generates, as do first-party ones.
"$root/target/debug/tok" plugin new "$temporary/device-tools" --platforms ios,macos > /dev/null
for registration in "$temporary/device-tools/apple/Exports.swift" "$root"/plugins/*/apple/Exports.swift; do
  checked=$(cd "${registration%/apple/Exports.swift}" && TOKAMAK_PLATFORM_PACK_PATH=$packs "$root/target/debug/tok" plugin check 2>&1) ||
    { echo "$checked" >&2; exit 1; }
  # The stand-in pack describes apple plugins, so they are checked, not skipped.
  if [[ $checked == *"apple files are not checked"* ]]; then
    echo "$checked" >&2
    exit 1
  fi
done
