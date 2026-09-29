# Vendored libsqlite3-sys

This directory is the published `libsqlite3-sys` **0.38.2** crate, consumed
through `[patch.crates-io]` in the workspace `Cargo.toml`. Its build script and
bindings are unchanged. The differences from the published crate:

- `sqlite3/sqlite3.c`, `sqlite3/sqlite3.h` and `sqlite3/sqlite3ext.h` are
  generated from SQLite **3.53.4**, the release workerd builds D1 with.
- `patches/` holds the workerd patches applied to SQLite's source before
  generation, unmodified from workerd v1.20260825.1 (Apache-2.0):
  - `0001-row-counts-plain.patch` adds the per-statement row counters D1
    reports as `rows_read` and `rows_written`.
  - `0004-authorizer-rename-to-destination-name.patch` passes a table
    rename's destination to the authorizer, which refuses reserved names.
- The unused `sqlcipher/` directory is removed.

## Options

The compile options are `env.LIBSQLITE3_FLAGS` in the workspace's
`.cargo/config.toml`. `libsqlite3-sys` compiles `sqlite3.c` with them, and
generation passes the same options to SQLite's code generators, so options
that change the grammar, such as `SQLITE_ENABLE_UPDATE_DELETE_LIMIT`, take
effect. The release profile's `opt-level = "z"` compiles SQLite at `-Oz`.

Three options D1 refuses or does not expose stay compiled in because SQLite
uses their code: `SQLITE_OMIT_ATTACH` (index and trigger code need it),
`SQLITE_OMIT_TEMPDB` (`ALTER TABLE … RENAME` reads the temporary schema) and
`SQLITE_OMIT_INCRBLOB` (FTS5 uses blob I/O). Progress callbacks stay compiled
in for the D1 query time limit and schema version pragmas for D1 bookmarks.

## Regenerating

```sh
cargo run -p xtask -- sqlite
```

The command downloads the pinned source archive from sqlite.org, checks its
SHA-256, applies `patches/`, runs `configure` and `make sqlite3.c` with the
options, and writes the three files here. It needs `curl`, `unzip`, `patch`,
`make` and a C compiler. To move to another SQLite release, update the pin in
`tools/xtask/src/sqlite.rs` to the version workerd builds and replace
`patches/` with workerd's patches for it.

## Size

`tok build` links storage, and SQLite with it, into an app only when the app's
Wrangler configuration declares a storage binding. Linking it adds this much
to a release build:

| Target | Binary | Without storage (bytes) | With storage (bytes) | Added |
|---|---|---|---|---|
| Android arm64 | `libtokamak.so` | 9,314,288 | 10,491,840 | 1,150 KB |
| iOS Simulator arm64 | The app's executable | 8,528,864 | 9,521,936 | 970 KB |
| macOS arm64 | The app's executable | 8,277,872 | 9,270,992 | 970 KB |
