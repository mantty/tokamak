# Memory optimisation findings

Status: draft findings, not an implementation plan

## Observed macOS baseline

The foreground Astro example used 115.9 MB after excluding an unrelated iOS
Simulator process.

| Process | Footprint |
|---|---:|
| tokamak host, including Bare | 57.4 MB |
| WebContent | 31.4 MB |
| Graphics and Media | 18.8 MB |
| Networking | 8.3 MB |
| Total | 115.9 MB |

WebContent had peaked at 114.3 MB and returned to 31.4 MB. Graphics and Media
had peaked at 46 MB and returned to 18.8 MB. The host had peaked at 59.6 MB
and remained at 57.4 MB, making the host and Bare the clearest area to
investigate first.

## Bare and the host process

The host contains a separate JavaScriptCore VM for the Bare worker. Its memory
profile included 19.2 MB resident in JavaScriptCore's `WebKit Malloc`
allocator and 11.5 MB in the standard small-object allocator.

The same process had four idle libuv worker threads and loaded a 1.3 MB worker
bundle.

tokamak currently initializes Bare with zeroed worklet options. BareKit exposes
an `optimize_for_memory` mode which reduces the libuv thread pool from four
threads to one. The pinned JavaScriptCore adapter does not apply the engine
part of that setting, so enabling it alone would only produce a modest macOS
improvement. Bare's per-worklet `memory_limit` is also ignored by that adapter.

The profile does not yet establish:

- whether one libuv worker is sufficient for tokamak workloads;
- how much startup and memory-pressure garbage collection would release;
- why JavaScriptCore retains almost its entire peak footprint;
- the memory and performance trade-off from disabling JIT; or
- the memory and compatibility trade-off from a QuickJS Bare adapter.

A compatible QuickJS adapter could preserve tokamak's WebView and worker model,
but its compatibility and performance are currently unmeasured.

## Native plugins

The generated plugin registry constructs every native plugin when the WebView
bridge starts. The location plugin consequently creates a
`CLLocationManager` before the application requests location.

The generated registry could hold factories rather than instances, deferring
plugin construction until its first call. Native plugin frameworks could also
be loaded on demand. Both options would keep unused platform services out of
an application's idle footprint without changing application code.

## Compatibility modules

The runtime installs Fetch, crypto, buffers, streams, process compatibility,
WebSockets, and their dependencies during startup. The current packed worker
contains 192 files from 42 packages.

Cloudflare-compatible globals must remain observable from application startup,
but some implementations may be initializable through synchronous lazy
properties. Potential candidates are:

- WebCrypto algorithms;
- compression;
- Node process, TTY, and inspection support;
- outbound HTTPS and DNS; and
- WebSocket support when unused.

Any lazy implementation must preserve normal property and error semantics.

## Worker packaging

The current esbuild invocation does not minify the generated CommonJS worker.
Minifying the Astro example's `tokamak-worklet.cjs` reduced it from 639,863 bytes
to 367,171 bytes, a 42.6% reduction. The complete bundle contains about
1.14 MB of JavaScript source, so this is useful but cannot explain tens of
megabytes by itself.

The heap also contained several individual live allocations of approximately
1.2 to 1.3 MB. The current profile cannot establish whether they are duplicate
bundle representations because allocation backtraces were unavailable.

## WebKit

The WebKit helper processes shown in the baseline use its documented separate
UI, WebContent, Networking, and graphics architecture. The public
configuration does not expose controls to:

- merge or disable the GPU process;
- eliminate the Networking process;
- select a single-process mode; or
- set process-count or cache-memory limits.

`WKProcessPool` no longer affects process allocation. Persistent website
storage is required for normal browser semantics, and changing to an ephemeral
store would not remove the Networking process.

`WKInactiveSchedulingPolicy.suspend` may reduce the cost of a WebView detached
from its window. It does not affect the foreground measurement above, but is
relevant to hidden and background applications.

Unsupported WebKit preferences or private process switches are not suitable
for an App Store framework.

References:

- [WebKit process architecture](https://docs.webkit.org/Getting%20Started/Introduction.html)
- [WKWebViewConfiguration public API](https://github.com/WebKit/WebKit/blob/main/Source/WebKit/UIProcess/API/Cocoa/WKWebViewConfiguration.h)
- [WKWebsiteDataStore public API](https://github.com/WebKit/WebKit/blob/main/Source/WebKit/UIProcess/API/Cocoa/WKWebsiteDataStore.h)

## Evidence gaps

The current measurements do not isolate the native AppKit shell, a blank
WKWebView, an empty Bare worker, the tokamak compatibility runtime, and the Astro
application. They also do not show the effect of startup collection or system
memory pressure.

Comparisons require the same window size, display scale, operating system,
settling time, and process-footprint accounting. Electron figures must include
every application-owned process.

The current evidence does not support treating 115.9 MB as fixed overhead.
It also does not yet establish how much each candidate change would save.
