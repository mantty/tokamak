# tokamak codebase cleanup

Status: active; items 1, 2, 3, 5, 7, and 8 complete

This is a structural cleanup plan for the current QuickJS codebase. It does
not change the runtime contract or add compatibility features.

The `mod.rs` roots have already been reduced to module declarations and
re-exports. Item 1 is now complete; the items below are the remaining
structural smells.

## 1. Split the remaining monolithic implementation files

Several files are now correctly named but still own too many unrelated
responsibilities:

| File | Mixed responsibilities |
|---|---|
| `tokamak/src/fs/node/exports.rs` | module registration and export wiring, with filesystem operations, callbacks, JavaScript objects, and tests in neighbouring files |
| `tokamak/src/quickjs/gateway.rs` | runtime lifecycle, listener recovery, Worker loading, assets, and request execution, with transport and tests separated |
| `tokamak/src/fs/vfs/virtual_filesystem.rs` | public API and request-scoped tree operations, with nodes/devices, bundle storage, path handling, and tests separated |

Split each by responsibility without introducing new public API solely for the
refactor. A
reasonable direction is:

- `fs/node`: module/export registration, filesystem operations, callback and
  promise adapters, JS object types, and small conversion helpers;
- `quickjs`: lifecycle/gateway, HTTP, WebSocket, Worker loading, and assets;
- `fs/vfs`: public types, tree nodes/devices, bundle storage, path handling, and
  tests;

Do not create a module for a single helper merely to reduce a line count. A
new file should own a coherent responsibility and have a narrow visibility
boundary.

## 2. Move platform-pack recipes behind the platform boundary

`tools/xtask/src/builder.rs` is now a generic platform-pack orchestrator.
The public platform-pack API in `tokamak-cli/src/lib.rs` owns canonical target metadata;
platform recipes own toolchain setup, runtime packaging, shell source paths,
entrypoints, and required tools.

The maintainer tool should remain the generic platform-pack orchestrator. Each
platform should own the metadata and recipe needed to prepare its pack, while
the common tool should only:

1. select a target;
2. invoke the platform recipe;
3. copy the returned artifacts into the pack layout; and
4. write and validate the manifest.

This keeps platform-specific build knowledge beside the platform shell and
prevents the maintainer tool from becoming a second platform abstraction.

## 3. Remove duplicate platform/target mappings

The platform and target relationships now live in the private `platform_pack`
module inside the CLI library. The CLI and maintainer tooling consume
`Platform` and `Target` metadata instead of defining duplicate enums and
matches. The on-disk manifest and CLI command remain `platform-pack`.

## 4. Narrow the CLI build modules

`tokamak-cli/src/pipeline.rs` is the right orchestration layer, but
`tokamak-cli/src/support.rs` still combines:

- project/package discovery;
- package-manager selection;
- platform-pack validation;
- staging and recursive copying;
- output-directory naming; and
- entrypoint process execution.

Separate those concerns into small modules with names that describe their
ownership. The pipeline should read as a sequence of app preparation, platform
pack staging, and entrypoint invocation; it should not contain platform build
policy.

## 5. Centralise workspace and pack-layout paths

The platform-pack builder owns one `WorkspaceLayout` for repository and staging
paths. Platform recipes own their source paths, while platform-pack tests use the
same manifest and artifact path contract as production.

The builder uses one `WorkspaceLayout` for repository and pack paths, while
platform recipes resolve their own source files. Tests consume the same
manifest and entrypoint contract as production.

## 6. Reduce fixture-heavy integration tests

`tokamak-cli/tests/build.rs` contains fixture construction, fake platform-pack creation,
fake shell entrypoints, and nineteen build behaviours in one file.
`tokamak/tests/packaged_worker.rs` similarly combines runtime setup, certificate
material, HTTP framing, WebSocket framing, and assertions.

Move reusable builders and protocol fixtures into the existing
`tokamak-cli/tests/fixtures` and `tokamak/tests/fixtures` locations. Split tests by
behaviour only where that makes failures or ownership clearer. Keep the
platform entrypoint contract covered by a small shared fixture rather than
duplicating fake pack construction in every test.

## 7. Make platform source layout uniform

All platforms now put shell sources under `source/`. The source root is not
coupled to the implementation language.

Each platform keeps its shell source, platform-pack recipe, app build entrypoint,
and platform metadata under its own directory.

## 8. Keep the platform-pack API narrow

The manifest contract lives in `tokamak-cli/src/platform_pack.rs`, while the maintainer
build logic lives in `tools/xtask/src/builder.rs`. `tokamak-cli/src/lib.rs` re-exports
only the platform-pack types, constants, and functions needed by the CLI and
maintainer tooling; it does not expose the CLI's build modules.

The on-disk `platform-pack.json` name and CLI terminology are unchanged.

## 9. Audit visibility after the structural split

The roots now re-export only the items needed by neighbouring packages, but
the large implementation files still deserve a deliberate visibility pass.
In particular:

- keep VFS nodes, devices, path helpers, and filesystem internals private;
- expose only the native module definitions and installation seam from
  `fs/node`;
- keep Apple/Android bridge modules private because their ABI symbols are the
  interface, not Rust module paths; and
- retain public platform-pack types only where the CLI or maintainer tooling
  consumes them.

Prefer `pub(crate)` or private items over adding re-exports to make a moved
implementation compile.

## 10. Remove or archive stale architecture documents

Several active-looking documents still describe the removed Bare runtime,
including the dev-mode, certificate-hardening, request-realms, WebView proxy,
and memory drafts. `docs/bare-codebase-review.md` is explicitly historical,
but it still appears alongside current plans.

Mark obsolete plans as archived or rewrite them against QuickJS. Remove stale
Bare paths and claims from active documents, and link the current structure
from one canonical QuickJS architecture plan.

## Cleanup order

1. ~~Split VFS and native `node:fs` around their existing public boundaries.~~
2. ~~Split the QuickJS gateway and certificate bundle implementations.~~
3. ~~Move platform-pack recipes into platform-owned descriptors and centralise
   pack paths.~~
4. ~~Collapse the duplicate target/platform mapping.~~ Narrow CLI support.
5. Reorganise fixtures and narrow the remaining CLI build modules.
6. Archive stale documentation and perform the final visibility audit.

Each step is complete when formatting, clippy, workspace tests, and the
affected platform/packaging tests remain green with no new public API added
solely to bridge the refactor.
