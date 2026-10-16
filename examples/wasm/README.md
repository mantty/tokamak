# Wasm Worker

The included `compute.wasm` exports a Rust addition function. The same Worker
returns `{"answer":42}` in local development and in a packaged Tokamak app.

```sh
pnpm install
pnpm dev
```

To package it, build a platform pack from this branch using the repository's
contributor workflow, then run `pnpm tokamak:macos` with
`TOKAMAK_PLATFORM_PACK_PATH` pointing at `target/tokamak-platform-packs`.

The prebuilt Wasm file needs no Rust tooling to consume. Contributors can rebuild
it with the `wasm32-unknown-unknown` Rust target:

```sh
rustc --target wasm32-unknown-unknown --crate-type cdylib -C opt-level=z -C panic=abort compute.rs -o compute.wasm
```

[Runtime support and limits](../../docs/wasm.md).
