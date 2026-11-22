#!/usr/bin/env bash
# Runs the location plugin's tests in a package that depends on the plugin,
# whose dependency on the plugin API resolves once the plugins are installed.
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)
temporary=$(mktemp -d)
trap 'rm -rf "$temporary"' EXIT

sources=$temporary/Sources/LocationTests
mkdir -p "$sources"
cp "$root/platforms/apple/tests/location.swift" "$sources/"
cat > "$temporary/Package.swift" <<EOF
// swift-tools-version: 5.9

import PackageDescription

let package = Package(
  name: "LocationTests",
  platforms: [.macOS("14.0")],
  dependencies: [
    .package(path: "$root/plugins/core/apple/TokamakPlugin"),
    .package(name: "TokamakLocation", path: "$root/plugins/location/apple"),
  ],
  targets: [
    .executableTarget(
      name: "LocationTests",
      dependencies: [
        .product(name: "TokamakPlugin", package: "TokamakPlugin"),
        .product(name: "TokamakLocation", package: "TokamakLocation"),
      ])
  ]
)
EOF
swift run --quiet --package-path "$temporary"
