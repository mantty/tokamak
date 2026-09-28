import Foundation
import Network
import Security
import WebKit

#if os(macOS)
  import AppKit
#else
  import UIKit
#endif

private let failurePage =
  "<h1>App failed to start</h1><p>Check startup-error.log for details.</p>"

private final class NavigationDelegate: NSObject, WKNavigationDelegate, WKUIDelegate {
  private let runtime: RuntimeHandle
  private let pluginBridge: TokamakPluginBridge

  init(runtime: RuntimeHandle, pluginBridge: TokamakPluginBridge) {
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

    if url.isAppOrigin(runtime.appHost) {
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

    if url.isAppOrigin(runtime.appHost) {
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
    if url.isAppOrigin(runtime.appHost) {
      webView.load(navigationAction.request)
    } else {
      openExternal(url)
    }
    return nil
  }

  /// The app's own usage descriptions cover its origin; other origins keep
  /// WebKit's prompt.
  func webView(
    _ webView: WKWebView,
    requestMediaCapturePermissionFor origin: WKSecurityOrigin,
    initiatedByFrame frame: WKFrameInfo,
    type: WKMediaCaptureType,
    decisionHandler: @escaping (WKPermissionDecision) -> Void
  ) {
    decisionHandler(origin.isAppOrigin(runtime.appHost) ? .grant : .prompt)
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
  private let host: TokamakHost
  private var runtime: RuntimeHandle?
  private var navigation: NavigationDelegate?
  private var pluginBridge: TokamakPluginBridge?
  private weak var activeWebView: WKWebView?
  private var dataStore: WKWebsiteDataStore?
  private var proxyPort: UInt16?

  init(host: TokamakHost) {
    self.host = host
  }

  func start(
    frame: CGRect,
    completion: @escaping (WKWebView) -> Void
  ) {
    host.whenStarted { result in
      switch result {
      case .success(let runtime):
        self.runtime = runtime
        completion(self.webView(frame: frame, runtime: runtime))
      case .failure:
        completion(self.failureWebView(frame: frame))
      }
    }
  }

  func restoreGateway() {
    guard let runtime, let dataStore else { return }
    do {
      let port = try runtime.restoreGateway()
      guard port != proxyPort else { return }
      setProxy(port: port, host: runtime.appHost, dataStore: dataStore)
      proxyPort = port
      activeWebView?.reload()
    } catch {
      print("tokamak gateway could not recover: \(error)")
    }
  }

  private func webView(frame: CGRect, runtime: RuntimeHandle) -> WKWebView {
    let configuration = WKWebViewConfiguration()
    let dataStore = WKWebsiteDataStore.default()
    let port = runtime.port
    setProxy(port: port, host: runtime.appHost, dataStore: dataStore)
    self.dataStore = dataStore
    self.proxyPort = port
    configuration.websiteDataStore = dataStore
    // Video plays within the page, and media starts without a tap.
    #if os(iOS)
      configuration.allowsInlineMediaPlayback = true
    #endif
    configuration.mediaTypesRequiringUserActionForPlayback = []

    let pluginBridge = TokamakPluginBridge(
      host: runtime.appHost,
      plugins: host.plugins
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
    webView.load(URLRequest(url: URL(string: "https://\(runtime.appHost)/")!))
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
    private lazy var host = TokamakHost()
    private lazy var controller = TokamakController(host: host)
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

    func application(
      _ application: NSApplication,
      didRegisterForRemoteNotificationsWithDeviceToken deviceToken: Data
    ) {
      host.didRegisterForRemoteNotifications(deviceToken: deviceToken)
    }

    func application(
      _ application: NSApplication,
      didFailToRegisterForRemoteNotificationsWithError error: Error
    ) {
      host.didFailToRegisterForRemoteNotifications(error: error)
    }

    func application(
      _ application: NSApplication,
      didReceiveRemoteNotification userInfo: [String: Any]
    ) {
      host.didReceiveRemoteNotification(userInfo) { _ in }
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
    private lazy var host = TokamakHost()
    private lazy var controller = TokamakController(host: host)
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

    func application(
      _ application: UIApplication,
      didRegisterForRemoteNotificationsWithDeviceToken deviceToken: Data
    ) {
      host.didRegisterForRemoteNotifications(deviceToken: deviceToken)
    }

    func application(
      _ application: UIApplication,
      didFailToRegisterForRemoteNotificationsWithError error: Error
    ) {
      host.didFailToRegisterForRemoteNotifications(error: error)
    }

    func application(
      _ application: UIApplication,
      didReceiveRemoteNotification userInfo: [AnyHashable: Any],
      fetchCompletionHandler completionHandler: @escaping (UIBackgroundFetchResult) -> Void
    ) {
      host.didReceiveRemoteNotification(userInfo) { result in
        switch result {
        case .newData: completionHandler(.newData)
        case .noData: completionHandler(.noData)
        case .failed: completionHandler(.failed)
        }
      }
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
