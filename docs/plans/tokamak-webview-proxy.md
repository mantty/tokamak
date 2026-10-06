# tokamak WebView transport and proxy plan

Status: proposal

## Objective

Give every tokamak application a stable HTTPS origin while keeping the transport
implementation appropriate to each WebView platform.

The application host is `<appname>.tokamak.local`. The host is part of the certificate
SAN, request URL, WebSocket URL, and Web Platform origin. Applications do not
configure or import any transport code.

App names are canonicalized to one lower-case DNS label of at most 63 bytes,
containing only `a-z`, `0-9`, and interior hyphens. Leading or trailing
hyphens are rejected. A Wrangler `name` is required. Without a `name` setting
the Worker name is the app name, and one that is not a valid slug is a
packaging error.

## Required invariants

- The tokamak origin is `https://<appname>.tokamak.local/` with no runtime-selected port.
- Bare receives app traffic directly in-process. There is no second loopback
  connection between a proxy and Bare.
- The transport never changes the HTTP, streaming, WebSocket, cookie, or
  service-worker semantics exposed to the application.
- Requests for non-tokamak hosts do not enter the tokamak proxy when the WebView can
  selectively match proxy domains.
- If a platform cannot selectively match domains, the proxy is a complete
  forward proxy: tokamak hosts go to Bare in-process and every other host is
  forwarded normally without TLS termination or request rewriting.
- mTLS is used on every proxy-facing connection that is reachable from a
  device/network interface or otherwise cannot be restricted to the app's
  WebView. Loopback-only transports may use server TLS without client auth
  when the platform boundary is sufficient.
- No password-based proxy authentication is used.
- Native shells keep one app WebView: same-origin top-level navigations stay in
  that view, while external top-level navigations go to the operating system;
  subframes and page resources remain unrestricted.

## Transport modes

Each platform adapter selects one mode from its capabilities rather than
sharing assumptions about WebView networking.

### In-process WebView transport

Use this when the platform exposes a request/stream interception API that can
preserve HTTPS origins, streaming bodies, WebSockets, and third-party network
requests without a network listener.

- No proxy port is allocated.
- No loopback socket is opened.
- mTLS is not needed for the tokamak transport boundary.
- The adapter calls the structured `WorkerDispatcher` contract directly.
- The platform may select this mode only if its request, response, streaming,
  and WebSocket APIs preserve the same semantics as the network path.

### Loopback proxy transport

Use this otherwise.

- Bind the proxy to `127.0.0.1:0` and use the assigned port only for the
  WebView's proxy configuration.
- The WebView loads `https://<appname>.tokamak.local/`; the proxy port is not part of
  the origin and may change every launch.
- Route the tokamak host to Bare through the common in-process connection
  handler, not through another TCP listener.
- Enable mTLS from WebView to the proxy when the platform exposes the proxy
  beyond a loopback-only boundary or cannot guarantee that only the app's
  WebView can connect.
- If the proxy is loopback-only and the platform boundary is sufficient, use
  server TLS without client authentication.

## Common runtime transport contract

The runtime must separate transport from Worker semantics.

The `WorkerDispatcher` owns:

- conversion to the tokamak `Request` and environment;
- streaming request and response bodies with backpressure;
- Worker response headers and status;
- WebSocket upgrade and frame delivery at the Worker boundary.

The `HTTPConnectionAdapter` owns HTTP/TLS framing, connection shutdown, and
error propagation. It accepts an already-established duplex stream, feeds the
`WorkerDispatcher`, and never binds a port itself. The existing `listen()` path
becomes a thin transport adapter around it.

The loopback proxy uses `HTTPConnectionAdapter`. An in-process WebView adapter
uses `WorkerDispatcher` directly because WebView interception APIs normally
provide structured requests rather than raw sockets.

The proxy/gateway owns only:

- proxy protocol parsing;
- tokamak-host routing;
- optional TLS and client-certificate verification at the proxy boundary;
- direct forwarding of non-tokamak hosts when all-traffic proxying is required.

No platform adapter may implement Worker request, response, or WebSocket
semantics independently.

## Proxy routing

When selective matching is available, configure the WebView data store with a
proxy match for exactly `<appname>.tokamak.local`. External requests bypass the proxy.

When selective matching is unavailable, the proxy accepts all traffic:

- `<appname>.tokamak.local`: terminate the tokamak TLS boundary as configured, then call
  Bare in-process;
- any other host: establish a direct upstream connection and tunnel CONNECT
  traffic, or forward ordinary HTTP/1.1 requests when the WebView uses them;
- never present the tokamak certificate for an external host;
- never send external traffic to the Worker handler.

The proxy must set failover behavior so a tokamak request cannot silently bypass
the tokamak route and resolve externally.

## Platform separation

Define one discriminated native transport choice:

```text
TransportMode {
  InProcess,
  LoopbackProxy {
    selective_domains: bool,
    requires_client_auth: bool,
  },
}
```

`requires_client_auth` is true unless the platform adapter can demonstrate
both that the listener is bound exclusively to `127.0.0.1` and that untrusted
processes cannot reach that loopback endpoint. Binding to loopback alone is
not sufficient on a platform where another process can connect to it.

The common runtime consumes the selected transport mode and the dispatcher. It
does not import AppKit, UIKit, or WebKit types.

Platform adapters own:

- WebView configuration and proxy setup;
- platform authentication challenges and certificate identity creation;
- lifecycle and teardown of the selected transport;
- mapping the app name to the platform's navigation URL.

The macOS and iOS adapters may choose different modes and must be tested
independently. A capability discovered on one platform must not become a
cross-platform assumption.

## Certificate model

The server certificate contains `DNS:<appname>.tokamak.local`. Certificates are minted
in memory or in the existing app-private cache and are not stored in the
system keychain.

When proxy mTLS is required, the WebView presents the tokamak client identity to
the proxy. The proxy verifies the issuing CA before dispatching the connection
to Bare. There is no client-certificate hop between the proxy and Bare because
that boundary is in-process.

## Deferred alternative: proxy-only mTLS with an HTTP app origin

Keep this as an explicit alternative to the HTTPS design above:

- The WebView loads `http://<appname>.tokamak.local/`.
- The WebView-to-proxy connection uses mTLS, with the proxy terminating and
  validating that connection.
- The proxy dispatches the resulting HTTP request directly to Bare in-process.
- The proxy-to-Bare path has no socket, TLS session, or client certificate.

This protects the local transport boundary without requiring `bare-tls` or an
app-domain server certificate. It does not make the WebView origin secure:
`http://<appname>.tokamak.local` may not receive secure-context Web APIs, and its
browser-visible origin semantics differ from the current HTTPS contract.

Do not adopt this alternative unless the reduced web-platform surface is an
intentional tokamak decision and the platform proxy APIs can provide the required
WebView-to-proxy mTLS.

## Implementation sequence

1. Extract the current Bare HTTP/TLS server's connection-independent handler.
2. Add the generic tokamak gateway interface and routing rules.
3. Implement the loopback proxy on macOS, including selective app-host
   matching and direct handling of external hosts in an all-traffic mode.
4. Configure the macOS WebView for `https://<appname>.tokamak.local/` and persistent
   website data.
5. Add the iOS adapter independently, selecting its supported transport mode.
6. Add certificate SAN and proxy-mTLS coverage for every mode that requires
   it.
7. Test origin persistence, streaming, WebSockets, third-party requests,
   proxy bypass/failover, and app-host isolation on every platform.

## Acceptance criteria

- Repeated launches may choose different loopback proxy ports without changing
  `location.origin` or losing website data.
- No tokamak request reaches a second Bare TCP listener.
- External requests work without passing through the tokamak handler.
- A platform requiring a network-reachable proxy rejects connections without a
  valid client certificate.
- An app cannot accidentally route its tokamak origin through ordinary DNS or a
  direct proxy failover path.
- The example Astro application requires no transport-specific code or
  configuration.
