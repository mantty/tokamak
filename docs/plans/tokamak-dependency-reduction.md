# tokamak dependency and compile-time reduction

Status: draft

This plan reduces Rust dependency activation and CI compile time without
removing platform-specific lint or test coverage. It addresses dependency
scope before changing CI commands because target and feature duplication is
only useful to assess after the package graph is correct.

## Baseline

The workspace members directly declare a modest number of dependencies, but
native libraries, build dependencies, procedural macros, tests, and platform
targets expand the resolved graph substantially.

| Build graph | Approximate unique packages |
|---|---:|
| Base `tokamak` on Linux | 69 |
| Native `tokamak` on Linux | 110 |
| Native Android `tokamak` | 119 |
| Native iOS `tokamak` | 138 |
| `tokamak-cli` with Linux test dependencies | 140 |
| Windows shell | 172 |

These `cargo tree` counts are not compile-unit counts. Cargo can compile one
package more than once for different targets, feature sets, build scripts, or
procedural-macro hosts. The CI lint commands also build test dependencies
through `--all-targets`.

The main expensive branches are:

- `rquickjs`, including the bundled QuickJS C build, bindgen, and its
  procedural macro;
- vendored OpenSSL, compiled from source for each target triple;
- `rcgen`, `ring`, `x509-parser`, and `p256`;
- `tao`, `wry`, WebView2 COM bindings, and Windows API crates; and
- test-only crates pulled in by `--all-targets`.

## Constraints

- Keep Linux, Android, Apple, and Windows lint coverage.
- Keep native platform tests on their required hosts.
- Preserve the base `tokamak` library configuration used by `tokamak-cli`.
- Do not replace cryptographic implementations without equivalent certificate,
  mTLS, renewal, and platform-key tests.
- Measure each reduction independently so dependency-count changes and CI time
  changes remain attributable.

## 1. Remove unused workspace dependency entries

Remove workspace catalogue entries that no member manifest consumes:

- `block2`;
- `dispatch2`;
- `objc2`;
- `objc2-app-kit`;
- `objc2-core-foundation`;
- `objc2-foundation`;
- `objc2-security`;
- `objc2-ui-kit`; and
- `objc2-web-kit`.

This is manifest hygiene only. These entries are not in the resolved graph, so
their removal should not change `Cargo.lock` or CI duration.

## 2. Scope native platform dependencies to the native feature

`tokamak/src/apple_ffi.rs` and `tokamak/src/android_jni.rs` are compiled only with
the `native` feature, but their `p256` and `jni` dependencies are currently
enabled for every build of their respective target.

- Make `p256` and `jni` optional target dependencies.
- Enable them through the `native` feature.
- Confirm that base `tokamak` remains buildable for every supported host target.

On Apple hosts, unconditional `p256` activation accounts for nearly all of the
current difference between the base Linux and base macOS dependency graphs.

## 3. Minimise `p256` features

The Apple bridge parses a PKCS#8 private key, derives its public key, and emits
the X9.63 representation. The current `p256` defaults also enable ECDSA
signing, verification, PEM, SHA-256, and standard-library features.

Disable default features and retain only the features required for PKCS#8
decoding, public-key derivation, and SEC1 output. Verify the exact minimal set
by compiling and testing every supported Apple target rather than assuming it
from feature names.

## 4. Remove the `rquickjs` procedural macro

The `macro` feature is used by two `#[rquickjs::module]` declarations in the
native `node:fs` exports. The surrounding `NodeFsModule` and
`NodeFsPromisesModule` types already implement `ModuleDef` and can call the
existing declaration and evaluation functions directly.

- Replace the two generated module adapters with the existing explicit
  `ModuleDef` implementations.
- Remove the `macro` feature from `rquickjs`.
- Keep the module names, exports, and runtime installation contract unchanged.

This removes a procedural-macro dependency tree and avoids the additional host
build of `rquickjs-core` and `rquickjs-sys` that the macro currently requires.

## 5. Restrict bindgen to targets that need it

`rquickjs-sys` ships bindings for the supported Linux, macOS, and Windows host
targets. It does not ship the required Android and iOS bindings, so those
targets still require bindgen.

- Remove `bindgen` from the common `rquickjs` feature set.
- Enable it only for Android and iOS target dependencies.
- Keep the current Android NDK Clang arguments and Apple target checks.
- Verify that no desktop build reports that bundled bindings are unavailable.

The bindgen branch currently contains about 24 packages and also introduces
host/target duplicate compile units.

## 6. Narrow CLI platform dependencies

### Host address discovery

`netdev` is used only to select a reachable host address for physical iOS
development.

Prefer standard-library route selection if it preserves the current default
interface behaviour. Otherwise, make `netdev` a macOS-only dependency and
keep non-macOS physical-iOS requests rejected at the existing platform
boundary.

### iOS signing

`plist`, `sha1`, and `sha2` are used only by the Apple signing crate and its
tests. Keep that dependency graph behind the Apple platform-pack signing tool;
the CLI only retains the `tok certs` facade.

After platform scoping, inspect the remaining `sha2` 0.11 and RustCrypto 0.10
duplicate graph. Do not downgrade a cryptographic crate solely to make
`cargo tree --duplicates` shorter; align versions only when the supported API
and maintenance cost are equivalent.

## 7. Reassess the CI compile graph

Keep the platform lint jobs, then measure whether the remaining base/native
invocations compile meaningful distinct code:

- Linux currently lints the workspace, native `tokamak`, and Android `tokamak`;
- Apple lints `tokamak-cli` and four Apple runtime targets; and
- Windows lints `tokamak-cli` and the native Windows shell.

Do not merge commands merely because they share dependencies. Consolidate a
base/native invocation only when every feature-gated source path remains
linted and the base library configuration is still compiled elsewhere.

Record cold-cache and restored-cache durations for each lint step. Compiled
artifacts are target-specific, so cross-target Apple, Android, and Windows
work cannot share target code even when crate versions match.

## 8. Evaluate cryptography consolidation separately

The runtime uses OpenSSL for TLS, `rcgen` and `ring` for certificate generation,
`x509-parser` and `ring` for parsing and verification, and `p256` for Apple key
conversion. These dependencies overlap in capability but are not currently
unused.

Before consolidating them, map each dependency to the exact operations and
tests it owns. Proceed only if one existing stack can replace another with less
code and equivalent behaviour on Linux, Android, macOS, iOS, and Windows.
This investigation must not block the lower-risk feature and platform scoping
work.

## Verification

For each completed item:

1. compare the affected `cargo tree` count and duplicate graph with this
   baseline;
2. run formatting and Clippy for every affected host and target;
3. run the base `tokamak`, native runtime, CLI, platform-pack, and platform build
   tests affected by the dependency;
4. verify the `Validate` CI gate remains the required aggregate result; and
5. compare both cold-cache and restored-cache CI durations.

## Proposed order

1. Remove unused workspace catalogue entries.
2. Scope `p256` and `jni` to `native`, then minimise `p256` features.
3. Remove the two `rquickjs` macro uses.
4. Restrict bindgen to Android and iOS targets.
5. Narrow CLI platform dependencies.
6. Re-measure and simplify duplicate CI compile configurations where coverage
   is unchanged.
7. Decide whether cryptography consolidation has enough measured value to
   justify a separate implementation plan.
