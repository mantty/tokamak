import Foundation
import Network
import Security
import TokamakRuntime
import WebKit

#if os(macOS)
  import AppKit
#else
  import UIKit
#endif

private let startupErrorFile = "startup-error.log"
private let failurePage =
  "<h1>App failed to start</h1><p>Check startup-error.log for details.</p>"

private enum ShellError: Error, CustomStringConvertible {
  case configuration(String)
  case runtime(String)

  var description: String {
    switch self {
    case .configuration(let message), .runtime(let message):
      return message
    }
  }
}

private enum AuthenticationMaterial<Value> {
  case defaultHandling
  case cancel
  case use(Value)
}

private final class RuntimeHost {
  let host: String
  private var handle: UnsafeMutableRawPointer?

  init() throws {
    guard
      let host = Bundle.main.object(forInfoDictionaryKey: "TokamakHost") as? String,
      !host.isEmpty
    else {
      throw ShellError.configuration("TokamakHost is required")
    }
    self.host = host

    let state = try Self.stateDirectory()
    var error = [CChar](repeating: 0, count: 512)
    if let endpoint = Bundle.main.object(forInfoDictionaryKey: "TokamakDevEndpoint") as? String,
      let sessionToken = Bundle.main.object(forInfoDictionaryKey: "TokamakDevSessionToken")
        as? String
    {
      handle = state.path.withCString { statePath in
        host.withCString { host in
          endpoint.withCString { endpoint in
            sessionToken.withCString { sessionToken in
              error.withUnsafeMutableBufferPointer { error in
                tokamak_runtime_start_development(
                  statePath,
                  host,
                  endpoint,
                  sessionToken,
                  error.baseAddress,
                  error.count
                )
              }
            }
          }
        }
      }
    } else {
      let bundle = Bundle.main.resourceURL?.appendingPathComponent("app")
      guard let bundle else {
        throw ShellError.configuration("app bundle resources are unavailable")
      }
      handle = bundle.path.withCString { bundlePath in
        state.path.withCString { statePath in
          host.withCString { host in
            error.withUnsafeMutableBufferPointer { error in
              tokamak_runtime_start(
                bundlePath,
                statePath,
                host,
                error.baseAddress,
                error.count
              )
            }
          }
        }
      }
    }
    guard handle != nil else {
      throw ShellError.runtime(String(cString: error))
    }
    try? FileManager.default.removeItem(
      at: state.appendingPathComponent(startupErrorFile)
    )
  }

  deinit {
    if let handle {
      tokamak_runtime_stop(handle)
    }
  }

  var port: UInt16 {
    tokamak_runtime_port(handle)
  }

  func restoreGateway() throws -> UInt16 {
    var error = [CChar](repeating: 0, count: 512)
    let port = error.withUnsafeMutableBufferPointer { error in
      tokamak_runtime_restore_gateway(
        handle,
        error.baseAddress,
        error.count
      )
    }
    guard port != 0 else {
      throw ShellError.runtime(String(cString: error))
    }
    return port
  }

  func serverAuthority(
    host: String
  ) -> AuthenticationMaterial<Data> {
    var bytes = TokamakBytes(data: nil, len: 0)
    let decision = host.withCString {
      tokamak_runtime_server_authority(handle, $0, &bytes)
    }
    switch decision {
    case Int32(TOKAMAK_DECISION_DEFAULT):
      return .defaultHandling
    case Int32(TOKAMAK_DECISION_USE):
      defer { tokamak_bytes_free(bytes) }
      guard let data = bytes.data else { return .cancel }
      return .use(Data(bytes: data, count: bytes.len))
    default:
      return .cancel
    }
  }

  func clientIdentity(
    host: String,
    previousFailures: Int
  ) -> AuthenticationMaterial<(Data, Data)> {
    var identity = TokamakIdentity(
      certificate: TokamakBytes(data: nil, len: 0),
      private_key: TokamakBytes(data: nil, len: 0)
    )
    let decision = host.withCString {
      tokamak_runtime_client_identity(
        handle,
        $0,
        max(previousFailures, 0),
        &identity
      )
    }
    switch decision {
    case Int32(TOKAMAK_DECISION_DEFAULT):
      return .defaultHandling
    case Int32(TOKAMAK_DECISION_USE):
      defer { tokamak_identity_free(identity) }
      guard
        let certificate = identity.certificate.data,
        let privateKey = identity.private_key.data
      else {
        return .cancel
      }
      return .use(
        (
          Data(bytes: certificate, count: identity.certificate.len),
          Data(bytes: privateKey, count: identity.private_key.len)
        ))
    default:
      return .cancel
    }
  }

  static func recordStartupFailure(_ error: Error) {
    print("tokamak runtime startup failed: \(error)")
    guard let state = try? stateDirectory() else { return }
    try? String(describing: error).write(
      to: state.appendingPathComponent(startupErrorFile),
      atomically: true,
      encoding: .utf8
    )
  }

  private static func stateDirectory() throws -> URL {
    guard let identifier = Bundle.main.bundleIdentifier else {
      throw ShellError.configuration("CFBundleIdentifier is required")
    }
    let root = try FileManager.default.url(
      for: .cachesDirectory,
      in: .userDomainMask,
      appropriateFor: nil,
      create: true
    )
    let state = root.appendingPathComponent(identifier, isDirectory: true)
    try FileManager.default.createDirectory(
      at: state,
      withIntermediateDirectories: true
    )
    return state
  }
}

private final class NavigationDelegate: NSObject, WKNavigationDelegate, WKUIDelegate {
  private let runtime: RuntimeHost
  private let pluginBridge: TokamakPluginBridge

  init(runtime: RuntimeHost, pluginBridge: TokamakPluginBridge) {
    self.runtime = runtime
    self.pluginBridge = pluginBridge
  }

  func webView(
    _ webView: WKWebView,
    didStartProvisionalNavigation navigation: WKNavigation?
  ) {
    pluginBridge.close()
  }

  func webView(
    _ webView: WKWebView,
    decidePolicyFor navigationAction: WKNavigationAction,
    decisionHandler: @escaping (WKNavigationActionPolicy) -> Void
  ) {
    guard let url = navigationAction.request.url else {
      decisionHandler(.cancel)
      return
    }

    if navigationAction.targetFrame?.isMainFrame == false {
      decisionHandler(.allow)
      return
    }

    if isAppOrigin(url) {
      if navigationAction.targetFrame == nil {
        webView.load(navigationAction.request)
        decisionHandler(.cancel)
      } else {
        decisionHandler(.allow)
      }
      return
    }

    openExternal(url)
    decisionHandler(.cancel)
  }

  func webView(
    _ webView: WKWebView,
    decidePolicyFor navigationResponse: WKNavigationResponse,
    decisionHandler: @escaping (WKNavigationResponsePolicy) -> Void
  ) {
    guard
      navigationResponse.isForMainFrame,
      let url = navigationResponse.response.url
    else {
      decisionHandler(.allow)
      return
    }

    if isAppOrigin(url) {
      decisionHandler(.allow)
    } else {
      openExternal(url)
      decisionHandler(.cancel)
    }
  }

  func webView(
    _ webView: WKWebView,
    createWebViewWith configuration: WKWebViewConfiguration,
    for navigationAction: WKNavigationAction,
    windowFeatures: WKWindowFeatures
  ) -> WKWebView? {
    _ = (configuration, windowFeatures)
    guard let url = navigationAction.request.url else { return nil }
    if isAppOrigin(url) {
      webView.load(navigationAction.request)
    } else {
      openExternal(url)
    }
    return nil
  }

  func webView(
    _ webView: WKWebView,
    didReceive challenge: URLAuthenticationChallenge,
    completionHandler:
      @escaping (
        URLSession.AuthChallengeDisposition,
        URLCredential?
      ) -> Void
  ) {
    let space = challenge.protectionSpace
    switch space.authenticationMethod {
    case NSURLAuthenticationMethodServerTrust:
      answer(
        runtime.serverAuthority(host: space.host),
        trust: space.serverTrust,
        completion: completionHandler
      )
    case NSURLAuthenticationMethodClientCertificate:
      answer(
        runtime.clientIdentity(
          host: space.host,
          previousFailures: challenge.previousFailureCount
        ),
        completion: completionHandler
      )
    default:
      completionHandler(.performDefaultHandling, nil)
    }
  }

  func webView(
    _ webView: WKWebView,
    didFailProvisionalNavigation navigation: WKNavigation?,
    withError error: Error
  ) {
    print("tokamak WebView navigation failed: \(error)")
  }

  private func isAppOrigin(_ url: URL) -> Bool {
    guard
      url.scheme?.caseInsensitiveCompare("https") == .orderedSame,
      url.host?.caseInsensitiveCompare(runtime.host) == .orderedSame
    else {
      return false
    }
    return url.port == nil || url.port == 443
  }

  private func openExternal(_ url: URL) {
    #if os(iOS)
      UIApplication.shared.open(url)
    #else
      NSWorkspace.shared.open(url)
    #endif
  }

  private func answer(
    _ material: AuthenticationMaterial<Data>,
    trust: SecTrust?,
    completion:
      @escaping (
        URLSession.AuthChallengeDisposition,
        URLCredential?
      ) -> Void
  ) {
    switch material {
    case .defaultHandling:
      completion(.performDefaultHandling, nil)
    case .cancel:
      completion(.cancelAuthenticationChallenge, nil)
    case .use(let authority):
      guard
        let trust,
        let certificate = SecCertificateCreateWithData(
          nil,
          authority as CFData
        ),
        SecTrustSetAnchorCertificates(
          trust,
          [certificate] as CFArray
        ) == errSecSuccess,
        SecTrustSetAnchorCertificatesOnly(trust, true) == errSecSuccess
      else {
        completion(.cancelAuthenticationChallenge, nil)
        return
      }
      let queue = DispatchQueue.global(qos: .userInitiated)
      queue.async {
        SecTrustEvaluateAsyncWithError(trust, queue) { trust, trusted, _ in
          DispatchQueue.main.async {
            guard trusted else {
              completion(.cancelAuthenticationChallenge, nil)
              return
            }
            completion(.useCredential, URLCredential(trust: trust))
          }
        }
      }
    }
  }

  private func answer(
    _ material: AuthenticationMaterial<(Data, Data)>,
    completion: (
      URLSession.AuthChallengeDisposition,
      URLCredential?
    ) -> Void
  ) {
    switch material {
    case .defaultHandling:
      completion(.performDefaultHandling, nil)
    case .cancel:
      completion(.cancelAuthenticationChallenge, nil)
    case .use(let value):
      guard
        let identity = identity(
          certificate: value.0,
          privateKey: value.1
        )
      else {
        completion(.cancelAuthenticationChallenge, nil)
        return
      }
      completion(
        .useCredential,
        URLCredential(
          identity: identity,
          certificates: nil,
          persistence: .none
        )
      )
    }
  }

  private func identity(
    certificate: Data,
    privateKey: Data
  ) -> SecIdentity? {
    guard
      let certificate = SecCertificateCreateWithData(
        nil,
        certificate as CFData
      )
    else {
      return nil
    }
    let attributes: [CFString: Any] = [
      kSecAttrKeyType: kSecAttrKeyTypeECSECPrimeRandom,
      kSecAttrKeyClass: kSecAttrKeyClassPrivate,
      kSecAttrKeySizeInBits: 256,
    ]
    guard
      let key = SecKeyCreateWithData(
        privateKey as CFData,
        attributes as CFDictionary,
        nil
      )
    else {
      return nil
    }
    return SecIdentityCreate(nil, certificate, key)
  }
}

private final class TokamakController {
  private var runtime: RuntimeHost?
  private var navigation: NavigationDelegate?
  private var pluginBridge: TokamakPluginBridge?
  private weak var activeWebView: WKWebView?
  private var dataStore: WKWebsiteDataStore?
  private var proxyPort: UInt16?

  func start(
    frame: CGRect,
    completion: @escaping (WKWebView) -> Void
  ) {
    DispatchQueue.global(qos: .userInitiated).async {
      let result = Result { try RuntimeHost() }
      DispatchQueue.main.async {
        switch result {
        case .success(let runtime):
          self.runtime = runtime
          completion(self.webView(frame: frame, runtime: runtime))
        case .failure(let error):
          RuntimeHost.recordStartupFailure(error)
          completion(self.failureWebView(frame: frame))
        }
      }
    }
  }

  func restoreGateway() {
    guard let runtime, let dataStore else { return }
    do {
      let port = try runtime.restoreGateway()
      guard port != proxyPort else { return }
      setProxy(port: port, host: runtime.host, dataStore: dataStore)
      proxyPort = port
      activeWebView?.reload()
    } catch {
      print("tokamak gateway could not recover: \(error)")
    }
  }

  private func webView(frame: CGRect, runtime: RuntimeHost) -> WKWebView {
    let configuration = WKWebViewConfiguration()
    let dataStore = WKWebsiteDataStore.default()
    let port = runtime.port
    setProxy(port: port, host: runtime.host, dataStore: dataStore)
    self.dataStore = dataStore
    self.proxyPort = port
    configuration.websiteDataStore = dataStore
    // Video plays within the page, and media starts without a tap.
    #if os(iOS)
      configuration.allowsInlineMediaPlayback = true
    #endif
    configuration.mediaTypesRequiringUserActionForPlayback = []

    let pluginBridge = TokamakPluginBridge(
      host: runtime.host,
      plugins: tokamakPlugins()
    )
    pluginBridge.install(in: configuration.userContentController)
    let navigation = NavigationDelegate(
      runtime: runtime,
      pluginBridge: pluginBridge
    )
    self.navigation = navigation
    let webView = WKWebView(frame: frame, configuration: configuration)
    #if os(iOS)
      webView.scrollView.bounces = false
    #endif
    webView.allowsLinkPreview = false
    activeWebView = webView
    pluginBridge.webView = webView
    self.pluginBridge = pluginBridge
    webView.navigationDelegate = navigation
    webView.uiDelegate = navigation
    webView.load(URLRequest(url: URL(string: "https://\(runtime.host)/")!))
    return webView
  }

  private func setProxy(
    port: UInt16,
    host: String,
    dataStore: WKWebsiteDataStore
  ) {
    var proxy = ProxyConfiguration(
      httpCONNECTProxy: .hostPort(
        host: "127.0.0.1",
        port: NWEndpoint.Port(rawValue: port)!
      )
    )
    proxy.matchDomains = [host]
    proxy.allowFailover = false
    dataStore.proxyConfigurations = [proxy]
  }

  private func failureWebView(frame: CGRect) -> WKWebView {
    let webView = WKWebView(frame: frame)
    webView.loadHTMLString(failurePage, baseURL: nil)
    return webView
  }
}

#if os(macOS)
  private final class TokamakMacApplicationDelegate: NSObject, NSApplicationDelegate {
    private let controller = TokamakController()
    private var window: NSWindow?

    func applicationDidFinishLaunching(_ notification: Notification) {
      let window = NSWindow(
        contentRect: NSRect(x: 0, y: 0, width: 1024, height: 768),
        styleMask: [
          .titled,
          .closable,
          .miniaturizable,
          .resizable,
        ],
        backing: .buffered,
        defer: false
      )
      window.title =
        Bundle.main.object(
          forInfoDictionaryKey: "CFBundleName"
        ) as? String ?? "tokamak"
      window.center()
      window.makeKeyAndOrderFront(nil)
      NSApplication.shared.activate(ignoringOtherApps: true)
      self.window = window

      guard let content = window.contentView else { return }
      controller.start(frame: content.bounds) { webView in
        webView.autoresizingMask = [.width, .height]
        content.addSubview(webView)
      }
    }

    func applicationShouldTerminateAfterLastWindowClosed(
      _ sender: NSApplication
    ) -> Bool {
      true
    }
  }

  @main
  private enum TokamakApplication {
    static func main() {
      let application = NSApplication.shared
      let delegate = TokamakMacApplicationDelegate()
      application.setActivationPolicy(.regular)
      application.delegate = delegate
      application.run()
    }
  }
#else
  private final class TokamakIOSApplicationDelegate: UIResponder, UIApplicationDelegate {
    private let controller = TokamakController()
    var window: UIWindow?

    func application(
      _ application: UIApplication,
      didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]?
    ) -> Bool {
      let window = UIWindow(frame: UIScreen.main.bounds)
      let viewController = UIViewController()
      window.rootViewController = viewController
      window.makeKeyAndVisible()
      self.window = window

      controller.start(frame: viewController.view.bounds) { webView in
        webView.autoresizingMask = [
          .flexibleWidth,
          .flexibleHeight,
        ]
        viewController.view.addSubview(webView)
      }
      return true
    }

    func applicationWillEnterForeground(_ application: UIApplication) {
      controller.restoreGateway()
    }

  }

  @main
  private enum TokamakApplication {
    static func main() {
      UIApplicationMain(
        CommandLine.argc,
        CommandLine.unsafeArgv,
        nil,
        NSStringFromClass(TokamakIOSApplicationDelegate.self)
      )
    }
  }
#endif
