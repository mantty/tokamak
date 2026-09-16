# tokamak Agent Instructions

tokamak is a cross-platform app framework designed to allow developers to create apps for mobile, desktop, and web from a single web-native codebase using vanilla fullstack web semantics and frameworks.

It aims to be broadly compatible with Cloudflare workers. Any app written for tokamak should work without changes on Cloudflare workers (though not all apps written for Cloudflare workers will necessarily run on tokamak). It supports frameworks such as Astro, NextJS, Hono, etc - all via Vite/Cloudflare.

## Simple is better than complex

Flat code structures are preferrable to nested ones.
Clear and descriptive names are better than clever ones, even if they are longer.
Never go beyond four levels of indentation.
If you have loops within loops, consider if a different data structure would avoid it.
After writing any code, ask yourself if it could be simpler. If it could be, it should be.

## Separation of concerns matters. A lot.

Above all else, consider what any area of code should be concerned with. What it should own. What it should know.
Preserving separation of concerns to appropriate, meaningful boundaries makes future maintenance simpler and avoids spaghetti code. Be strict in this regard. Any exceptions require user confirmation.

## Comments and Documentation

Comments and docs state current facts, tersely. One clause, present tense, only where the code cannot say it itself.

Never write:

- Narration or persuasion ("built for exactly this scenario", "matching what production does")
- Evidence trails ("confirmed via...", "its source comment cites...")
- History ("previously...", "this replaces...")
- Alternatives not taken, or anything else the code doesn't do

Bad: "WebAssembly still works there through DrumBrake, V8's own wasm interpreter, built for exactly this scenario -- its source comment cites tvOS, where JIT is unavailable."
Good: "jitless WebAssembly is supported via V8's own wasm interpreter, DrumBrake."

## The vendored QuickJS engine (`rquickjs/`)

The JS engine is a vendored copy of the `rquickjs-sys` crate at the repo root
(`rquickjs/`), consumed through `[patch.crates-io]`. Its bundled QuickJS carries a
small, deliberately minimal patch. Stay as close to upstream as is absolutely
possible.

Do not add engine changes when the behaviour can be provided from our own code.
Before touching anything under `rquickjs/quickjs/`, provide the feature in
`tokamak/src/...` (a host Rust function or bundled JS) and prove that works.
Patch the engine only when a behaviour is genuinely impossible from outside it —
because it depends on engine-internal state or an internal code path that
user-reachable JS cannot observe or intercept — and it has been demonstrated,
not assumed, that no external seam exists.

Every existing engine change, why it is irreducible, and how to re-apply it on a
version bump is documented in `rquickjs/PATCH.md`. Any new change must be added
there with the same justification, and must keep the patch minimal: prefer moving
logic out of the engine over adding to it. Grep `TOKAMAK` in the vendored source
for inline markers.

## Contributor/User Boundary

tokamak has two distinct audiences:

- tokamak users: people using tokamak to build applications.
- tokamak contributors: people maintaining tokamak itself.

Both groups are 'developers', but with different needs.

Runtime compilation, rust tooling, infrastructure, SDK packaging, and
runtime-platform build support are contributor concerns.
Do not put those flows in the tokamak user CLI unless explicitly requested.
Do not conflate the needs of these two groups.
tokamak users should never have to consider the internals of tokamak and how it is made.
