#!/usr/bin/env bash
# Packages the CLI and platform-pack archives from the build matrix as @tokamakdev/tok and its optional dependencies.
# Usage: package-cli-npm.sh <version> <archive-dir> <output-dir>
set -euo pipefail

version="$1"
archives="$2"
output="$3"
npm_root="$(cd "$(dirname "$0")/../tokamak-cli/npm" && pwd)"
# npm platform package : CLI archive host
platforms=(
  darwin-arm64:macos-arm64
  darwin-x64:macos-x64
  linux-x64:linux-x64
  win32-x64:windows-x64
)

mkdir -p "$output"
for entry in "${platforms[@]}"; do
  platform="${entry%%:*}"
  host="${entry##*:}"
  package="$npm_root/platforms/$platform"
  rm -rf "$package/bin"
  mkdir -p "$package/bin"
  if [[ $platform == win32-* ]]; then
    unzip -q -o "$archives/tokamak-cli-$host.zip" -d "$package/bin"
    test -f "$package/bin/tok.exe"
  else
    tar -xzf "$archives/tokamak-cli-$host.tar.gz" -C "$package/bin"
    test -f "$package/bin/tok"
  fi
  npm pkg set "version=$version" --prefix "$package"
  npm pkg set "optionalDependencies.@tokamakdev/tok-$platform=$version" --prefix "$npm_root"
  npm pack --silent --pack-destination "$output" "$package"
done

for package in "$npm_root"/platform-packs/*/; do
  package="${package%/}"
  target="$(basename "$package")"
  rm -rf "${package:?}/$target"
  mkdir -p "$package/$target"
  tar -xzf "$archives/tokamak-platform-pack-$target.tar.gz" -C "$package/$target"
  test -f "$package/$target/platform-pack.json"
  npm pkg set "version=$version" --prefix "$package"
  npm pkg set "optionalDependencies.@tokamakdev/platform-$target=$version" --prefix "$npm_root"
  npm pack --silent --pack-destination "$output" "$package"
done

npm pkg set "version=$version" --prefix "$npm_root"
npm pack --silent --pack-destination "$output" "$npm_root"
