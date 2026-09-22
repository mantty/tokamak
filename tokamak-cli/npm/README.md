# @tokamakdev/tok

The tokamak CLI, published from the tokamak workspace. The version is the
workspace Cargo version, with a `-beta.N` suffix for automated pre-releases.

```sh
npm install -g @tokamakdev/tok@beta
tok targets
```

The package installs the `tok` binary for the current platform through an
optional dependency (`@tokamakdev/tok-darwin-arm64`, `-darwin-x64`,
`-linux-x64`, or `-win32-x64`). Building apps also needs the target packs,
which the tokamak installer places under `~/.local/share/tokamak`; `tok`
finds them there, or wherever `TOKAMAK_TARGET_PACK_DIR` points.
