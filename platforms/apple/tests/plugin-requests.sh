#!/usr/bin/env bash
# Runs the plugin request tests, with the shell sources they cover, in a
# package that depends on the plugin API.
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)
apple=$root/platforms/apple
temporary=$(mktemp -d)
trap 'rm -rf "$temporary"' EXIT

sources=$temporary/Sources/PluginRequestsTests
mkdir -p "$sources"
cp "$apple/source/TokamakOrigin.swift" "$apple/source/TokamakControls.swift" "$apple/tests/plugin-requests.swift" \
  "$sources/"
cat > "$temporary/Package.swift" <<EOF
// swift-tools-version: 5.9

import PackageDescription

let package = Package(
  name: "PluginRequestsTests",
  platforms: [.macOS("14.0")],
  dependencies: [.package(path: "$root/plugins/core/apple/TokamakPlugin")],
  targets: [
    .executableTarget(
      name: "PluginRequestsTests",
      dependencies: [.product(name: "TokamakPlugin", package: "TokamakPlugin")])
  ]
)
EOF
swift run --quiet --package-path "$temporary"
