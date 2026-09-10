#!/usr/bin/env bash
set -euo pipefail

repository="mantty/tokamak"
api="https://api.github.com/repos/$repository"
targets=(
  android-arm64
  ios-arm64
  ios-simulator-arm64
  ios-simulator-x64
  macos-arm64
  macos-x64
  windows-x64
)

fail() {
  printf 'tokamak installer: %s\n' "$1" >&2
  exit 1
}

for command in curl tar mktemp install; do
  command -v "$command" > /dev/null || fail "$command is required"
done
[[ -n ${HOME:-} ]] || fail 'HOME is not set'

case "$(uname -s)/$(uname -m)" in
  Darwin/arm64) cli_host=macos-arm64 ;;
  Darwin/x86_64) cli_host=macos-x64 ;;
  Linux/x86_64 | Linux/amd64) cli_host=linux-x64 ;;
  *) fail "unsupported host: $(uname -s) $(uname -m)" ;;
esac

api_headers=(
  -H 'Accept: application/vnd.github+json'
  -H 'X-GitHub-Api-Version: 2022-11-28'
)
github_token="${GH_TOKEN:-${GITHUB_TOKEN:-}}"
if [[ -n $github_token ]]; then
  api_headers+=(-H "Authorization: Bearer $github_token")
fi
download_headers=(-H 'Accept: application/octet-stream')

release="$(curl -fsSL "${api_headers[@]}" "$api/releases?per_page=1")" || fail 'could not find a tokamak release'
tag="$(printf '%s\n' "$release" | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p')"
[[ -n $tag ]] || fail 'could not read the latest tokamak release'
[[ $tag =~ ^[A-Za-z0-9._-]+$ ]] || fail "unsupported release tag: $tag"

release_url="https://github.com/$repository/releases/download/$tag"
temporary="$(mktemp -d "${TMPDIR:-/tmp}/tokamak-install.XXXXXXXX")"
staged_cli=
staged_target_packs=
cleanup() {
  rm -rf "$temporary"
  [[ -z $staged_cli ]] || rm -f "$staged_cli"
  [[ -z $staged_target_packs ]] || rm -rf "$staged_target_packs"
}
trap cleanup EXIT

printf 'Downloading tokamak %s for %s...\n' "$tag" "$cli_host"
mkdir -p "$temporary/cli" "$temporary/target-packs"
cli_archive="$temporary/tokamak-cli.tar.gz"
curl -fsSL "${download_headers[@]}" "$release_url/tokamak-cli-$cli_host.tar.gz" -o "$cli_archive"
tar -xzf "$cli_archive" -C "$temporary/cli"
[[ -f $temporary/cli/tok ]] || fail 'CLI archive does not contain tok'

for target in "${targets[@]}"; do
  printf 'Downloading target pack %s...\n' "$target"
  archive="$temporary/tokamak-target-pack-$target.tar.gz"
  destination="$temporary/target-packs/$target"
  mkdir -p "$destination"
  curl -fsSL "${download_headers[@]}" "$release_url/tokamak-target-pack-$target.tar.gz" -o "$archive"
  tar -xzf "$archive" -C "$destination"
  [[ -f $destination/target-pack.json ]] || fail "$target archive does not contain target-pack.json"
done

bin_dir="$HOME/.local/bin"
share_dir="$HOME/.local/share/tokamak"
target_pack_dir="$share_dir/target-packs"
mkdir -p "$bin_dir" "$share_dir"

staged_cli="$bin_dir/.tokamak-install-$$"
staged_target_packs="$share_dir/.target-packs-install-$$"
install -m 755 "$temporary/cli/tok" "$staged_cli"
cp -R "$temporary/target-packs" "$staged_target_packs"
rm -rf "$target_pack_dir"
mv "$staged_target_packs" "$target_pack_dir"
staged_target_packs=
mv -f "$staged_cli" "$bin_dir/tok"
staged_cli=

printf '\nInstalled tokamak %s in %s\n' "$tag" "$bin_dir"
printf 'Installed target packs in %s\n' "$target_pack_dir"
case ":${PATH:-}:" in
  *":$bin_dir:"*) ;;
  *)
    printf 'Add %s to PATH in your shell profile:\n' "$bin_dir"
    printf '  export PATH="%s:%s"\n' "$bin_dir" "\$PATH"
    ;;
esac
printf 'Run tok targets to verify the installation.\n'
