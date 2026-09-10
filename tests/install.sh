#!/usr/bin/env bash
set -euo pipefail

repository_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
temporary="$(mktemp -d)"
trap 'rm -rf "$temporary"' EXIT

fixtures="$temporary/fixtures"
mkdir -p "$fixtures/cli" "$temporary/fake-bin"
printf '#!/usr/bin/env bash\necho tokamak\n' > "$fixtures/cli/tok"
chmod +x "$fixtures/cli/tok"
for cli_host in macos-arm64 macos-x64 linux-x64; do
  tar -czf "$fixtures/tokamak-cli-$cli_host.tar.gz" -C "$fixtures/cli" tok
done

targets=(
  android-arm64
  ios-arm64
  ios-simulator-arm64
  ios-simulator-x64
  macos-arm64
  macos-x64
  windows-x64
)
for target in "${targets[@]}"; do
  pack="$temporary/$target"
  mkdir -p "$pack"
  printf '{"target":"%s"}\n' "$target" > "$pack/target-pack.json"
  tar -czf "$fixtures/tokamak-target-pack-$target.tar.gz" -C "$pack" .
done

cat > "$temporary/fake-bin/curl" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
output=
url=
authorization=
while (($#)); do
  case "$1" in
    -H)
      if [[ ${2:-} == Authorization:* ]]; then
        authorization=$2
      fi
      shift 2
      ;;
    -o)
      output=$2
      shift 2
      ;;
    -*)
      shift
      ;;
    *)
      url=$1
      shift
      ;;
  esac
done
printf '%s\n' "$url" >> "$TOKAMAK_INSTALL_LOG"
if [[ $url == *'/releases?per_page=1' ]]; then
  if [[ -n ${TOKAMAK_INSTALL_EXPECTED_AUTH:-} ]]; then
    test "$authorization" = "Authorization: Bearer $TOKAMAK_INSTALL_EXPECTED_AUTH"
  else
    test -z "$authorization"
  fi
  printf '[{"tag_name":"pre.2"}]\n'
else
  cp "$TOKAMAK_INSTALL_FIXTURES/${url##*/}" "$output"
fi
EOF

cat > "$temporary/fake-bin/uname" <<'EOF'
#!/usr/bin/env bash
case "$1" in
  -s) printf '%s\n' "$TOKAMAK_INSTALL_UNAME_S" ;;
  -m) printf '%s\n' "$TOKAMAK_INSTALL_UNAME_M" ;;
esac
EOF
chmod +x "$temporary/fake-bin/curl" "$temporary/fake-bin/uname"

run_install() {
  local os=$1
  local architecture=$2
  local cli_host=$3
  local github_token=${4:-}
  local home="$temporary/home-$cli_host"
  local download_log="$temporary/downloads-$cli_host"
  local output="$temporary/output-$cli_host"
  mkdir -p "$home/.local/bin" "$home/.local/share/tokamak/target-packs/obsolete"
  printf 'old\n' > "$home/.local/bin/tok"
  chmod +x "$home/.local/bin/tok"
  printf 'old\n' > "$home/.local/share/tokamak/target-packs/obsolete/target-pack.json"

  HOME="$home" \
  TOKAMAK_INSTALL_FIXTURES="$fixtures" \
  TOKAMAK_INSTALL_LOG="$download_log" \
  TOKAMAK_INSTALL_UNAME_S="$os" \
  TOKAMAK_INSTALL_UNAME_M="$architecture" \
  TOKAMAK_INSTALL_EXPECTED_AUTH="$github_token" \
  GH_TOKEN="$github_token" \
  GITHUB_TOKEN= \
  PATH="$temporary/fake-bin:$PATH" \
    bash "$repository_root/scripts/install.sh" > "$output"

  test -x "$home/.local/bin/tok"
  grep -F 'echo tokamak' "$home/.local/bin/tok" > /dev/null
  for target in "${targets[@]}"; do
    test -f "$home/.local/share/tokamak/target-packs/$target/target-pack.json"
    grep -F "/tokamak-target-pack-$target.tar.gz" "$download_log" > /dev/null
  done
  test ! -e "$home/.local/share/tokamak/target-packs/obsolete"
  grep -F "/tokamak-cli-$cli_host.tar.gz" "$download_log" > /dev/null
  grep -F 'Installed tokamak pre.2' "$output" > /dev/null
  grep -F "Add $home/.local/bin to PATH" "$output" > /dev/null
}

run_install Darwin arm64 macos-arm64 ci-token
run_install Darwin x86_64 macos-x64
run_install Linux x86_64 linux-x64
