# Runtime streams

`web-standard.mjs` is generated from `web-streams-polyfill` by
`scripts/vendor-web-streams.mjs`. `scripts/web-streams-workerd.patch` preserves
Workerd's terminal-reader/writer closed promises, last-branch tee cancellation
reason, and lock-free handling of an already-errored pipe source.

`web.mjs` binds these streams to Tokamak's native codecs and Workerd extensions.
Compression is incremental and write-driven; its readable side supports BYOB.

`node.mjs` is generated from `readable-stream` by
`scripts/vendor-readable-stream.mjs`. Both upstream licenses are included here.

The bundled-runtime differential suite tests these APIs against Workerd.
