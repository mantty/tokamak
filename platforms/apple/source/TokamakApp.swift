import Foundation
import TokamakRuntime

#if os(iOS)
  import UIKit
#endif

private let startupErrorFile = "startup-error.log"
/// How long the Worker has for `resume` and `suspend`.
private let lifecycleDeadline: TimeInterval = 10
#if os(iOS)
  /// The system gives an app about 30 seconds after it leaves the foreground.
  private let suspendDeadline: TimeInterval = 25
#endif

/// The tokamak runtime and native plugins of this process. Both outlive any
/// WebView, so plugins receive app events before a page loads and a Worker
/// event can run while no page is loaded.
final class TokamakApp {
  /// Every plugin, by ID.
  private(set) var plugins: [String: any TokamakPlugin] = [:]
  private var runtime: Result<RuntimeHandle, Error>?
  private var waiting: [(Result<RuntimeHandle, Error>) -> Void] = []
  /// The Worker's calls and subscriptions on the plugins.
  private lazy var workerRequests = TokamakPluginRequests<UInt64>(plugins: plugins) {
    [unowned self] id, result in
    whenStarted { started in
      if case .success(let runtime) = started {
        runtime.reply(id, result: result)
      }
    }
  }
  /// Delivers `resume` and `suspend` one at a time, in order.
  private let lifecycle = DispatchQueue(label: "tokamak.lifecycle")
  #if os(iOS)
    /// The scenes in the foreground.
    private var foregroundScenes = 0
  #endif

  /// Creates the plugins. Call it before launch finishes.
  init() {
    for plugin in tokamakPlugins(app: self) {
      precondition(plugins.updateValue(plugin, forKey: plugin.id) == nil, "Duplicate plugin ID")
    }
  }

  /// Starts the runtime in the background, which delivers `start` with
  /// `foreground`. Call it once, on the main thread.
  func start(foreground: Bool) {
    dispatchPrecondition(condition: .onQueue(.main))
    let plugins = workerPlugins
    DispatchQueue.global(qos: .userInitiated).async {
      let result = Result { try RuntimeHandle(foreground: foreground, plugins: plugins) }
      DispatchQueue.main.async { self.started(result) }
    }
  }

  /// The plugins as the runtime calls them, on its own threads. The app
  /// outlives the runtime, which holds it unretained.
  private var workerPlugins: TokamakPluginHandler {
    TokamakPluginHandler(
      context: Unmanaged.passUnretained(self).toOpaque(),
      call: { context, id, plugin, method, arguments in
        TokamakApp.receive(context, id, subscribe: false, plugin, method, arguments)
      },
      subscribe: { context, id, plugin, method, arguments in
        TokamakApp.receive(context, id, subscribe: true, plugin, method, arguments)
      },
      unsubscribe: { context, id in
        let app = Unmanaged<TokamakApp>.fromOpaque(context!).takeUnretainedValue()
        DispatchQueue.main.async { app.workerRequests.cancel(id) }
      }
    )
  }

  /// Runs a call or subscription the Worker made through `context`'s app.
  private static func receive(
    _ context: UnsafeMutableRawPointer?,
    _ id: UInt64,
    subscribe: Bool,
    _ plugin: UnsafePointer<CChar>?,
    _ method: UnsafePointer<CChar>?,
    _ arguments: UnsafePointer<CChar>?
  ) {
    let app = Unmanaged<TokamakApp>.fromOpaque(context!).takeUnretainedValue()
    let (plugin, method) = (String(cString: plugin!), String(cString: method!))
    let arguments =
      (try? JSONSerialization.jsonObject(
        with: Data(String(cString: arguments!).utf8), options: .fragmentsAllowed)) ?? NSNull()
    DispatchQueue.main.async {
      app.workerRequests.run(
        id, subscribe: subscribe, plugin: plugin, method: method, arguments: arguments)
    }
  }

  /// Calls `completion` on the main thread once the runtime has started or
  /// failed to start.
  func whenStarted(_ completion: @escaping (Result<RuntimeHandle, Error>) -> Void) {
    dispatchPrecondition(condition: .onQueue(.main))
    if let runtime {
      completion(runtime)
    } else {
      waiting.append(completion)
    }
  }

  /// Delivers the event `name` with `event`, JSON-serialisable, to the
  /// Worker's listeners on `queue`, and calls `completion` on the main thread
  /// with their reply, parsed from JSON, or nil. Fails unless they return
  /// within `timeout`, which includes runtime startup. Call it on the main
  /// thread.
  func emit(
    _ name: String,
    event: Any,
    timeout: TimeInterval,
    on queue: DispatchQueue = .global(qos: .userInitiated),
    completion: @escaping (Result<Any?, Error>) -> Void
  ) {
    let deadline = Date() + timeout
    withRuntime(on: queue, completion: completion) { runtime in
      try runtime.emit(name, event: event, timeout: deadline.timeIntervalSinceNow)
    }
  }

  /// Fetches what the app serves at `path` and calls `completion` on the main
  /// thread with the body. Fails unless it arrives within `timeout`, which
  /// includes runtime startup. Call it on the main thread.
  func fetch(
    _ path: String,
    timeout: TimeInterval,
    completion: @escaping (Result<Data, Error>) -> Void
  ) {
    let deadline = Date() + timeout
    withRuntime(on: .global(qos: .userInitiated), completion: completion) { runtime in
      try runtime.fetch(path, timeout: deadline.timeIntervalSinceNow)
    }
  }

  /// Runs `work` with the started runtime on `queue`, and calls `completion`
  /// on the main thread with its result.
  private func withRuntime<Value>(
    on queue: DispatchQueue,
    completion: @escaping (Result<Value, Error>) -> Void,
    work: @escaping (RuntimeHandle) throws -> Value
  ) {
    whenStarted { result in
      switch result {
      case .failure(let error):
        completion(.failure(error))
      case .success(let runtime):
        queue.async {
          let outcome = Result { try work(runtime) }
          DispatchQueue.main.async { completion(outcome) }
        }
      }
    }
  }

  /// Records the app moving into or out of the foreground once the runtime
  /// has started, and delivers `resume` or `suspend` when that changed its
  /// stage. Call it on the main thread.
  func moved(toForeground foreground: Bool) {
    let name = foreground ? "resume" : "suspend"
    #if os(iOS)
      // The system suspends the app soon after it leaves the foreground
      // unless a background task holds it there.
      var task = UIBackgroundTaskIdentifier.invalid
      let finish = {
        guard task != .invalid else { return }
        UIApplication.shared.endBackgroundTask(task)
        task = .invalid
      }
      if !foreground {
        task = UIApplication.shared.beginBackgroundTask(
          withName: "tokamak suspend", expirationHandler: finish)
      }
      let timeout = foreground ? lifecycleDeadline : suspendDeadline
    #else
      let finish = {}
      let timeout = lifecycleDeadline
    #endif
    whenStarted { started in
      guard case .success(let runtime) = started, runtime.setForeground(foreground) else {
        finish()
        return
      }
      self.emit(name, event: [String: Any](), timeout: timeout, on: self.lifecycle) { result in
        if case .failure(let error) = result {
          print("tokamak \(name) failed: \(error)")
        }
        finish()
      }
    }
  }

  #if os(iOS)
    /// Delivers `resume` as the first scene enters the foreground.
    func sceneWillEnterForeground() {
      foregroundScenes += 1
      if foregroundScenes == 1 {
        moved(toForeground: true)
      }
    }

    /// Delivers `suspend` as the last scene leaves the foreground.
    func sceneDidEnterBackground() {
      foregroundScenes -= 1
      if foregroundScenes == 0 {
        moved(toForeground: false)
      }
    }

    /// Whether a scene is in the foreground.
    var hasForegroundScene: Bool {
      foregroundScenes > 0
    }

    /// Whether the app is in the foreground, as the runtime records it, or
    /// until the runtime has started, whether a scene is in the foreground.
    /// Read it on the main thread.
    var isInForeground: Bool {
      guard case .success(let runtime) = runtime else { return hasForegroundScene }
      return runtime.isForeground
    }
  #endif

  func didRegisterForRemoteNotifications(deviceToken: Data) {
    for plugin in plugins.values {
      plugin.didRegisterForRemoteNotifications(deviceToken: deviceToken)
    }
  }

  func didFailToRegisterForRemoteNotifications(error: Error) {
    for plugin in plugins.values {
      plugin.didFailToRegisterForRemoteNotifications(error: error)
    }
  }

  /// Delivers a remote notification to every plugin, calling `completion` on
  /// the main thread once each has finished with it.
  func didReceiveRemoteNotification(
    _ userInfo: [AnyHashable: Any],
    completion: @escaping (TokamakBackgroundResult) -> Void
  ) {
    let group = DispatchGroup()
    var results: [TokamakBackgroundResult] = []
    for plugin in plugins.values {
      group.enter()
      plugin.didReceiveRemoteNotification(userInfo) { result in
        DispatchQueue.main.async {
          results.append(result)
          group.leave()
        }
      }
    }
    group.notify(queue: .main) {
      completion(TokamakBackgroundResult(combining: results))
    }
  }

  private func started(_ result: Result<RuntimeHandle, Error>) {
    if case .failure(let error) = result {
      RuntimeHandle.recordStartupFailure(error)
    }
    runtime = result
    let callbacks = waiting
    waiting.removeAll()
    for callback in callbacks {
      callback(result)
    }
  }
}

/// What a plugin did with a background event, reported to the system.
enum TokamakBackgroundResult {
  case newData
  case noData
  case failed

  /// New data when any plugin fetched some, otherwise a failure when any
  /// plugin failed.
  init(combining results: [Self]) {
    if results.contains(.newData) {
      self = .newData
    } else if results.contains(.failed) {
      self = .failed
    } else {
      self = .noData
    }
  }
}

enum RuntimeError: Error, CustomStringConvertible {
  case configuration(String)
  case runtime(String)

  var description: String {
    switch self {
    case .configuration(let message), .runtime(let message):
      return message
    }
  }
}

enum AuthenticationMaterial<Value> {
  case defaultHandling
  case cancel
  case use(Value)
}

/// A started tokamak runtime, stopped when released.
final class RuntimeHandle {
  let appHost: String
  private var handle: UnsafeMutableRawPointer?

  /// Starts the runtime, which delivers `start` with `foreground`, and whose
  /// Worker calls `plugins`.
  init(foreground: Bool, plugins: TokamakPluginHandler) throws {
    guard
      let appHost = Bundle.main.object(forInfoDictionaryKey: "TokamakHost") as? String,
      !appHost.isEmpty
    else {
      throw RuntimeError.configuration("TokamakHost is required")
    }
    self.appHost = appHost

    let state = try Self.stateDirectory()
    var error = [CChar](repeating: 0, count: 512)
    if let endpoint = Bundle.main.object(forInfoDictionaryKey: "TokamakDevEndpoint") as? String,
      let sessionToken = Bundle.main.object(forInfoDictionaryKey: "TokamakDevSessionToken")
        as? String
    {
      handle = state.path.withCString { statePath in
        appHost.withCString { appHost in
          endpoint.withCString { endpoint in
            sessionToken.withCString { sessionToken in
              error.withUnsafeMutableBufferPointer { error in
                tokamak_runtime_start_development(
                  statePath,
                  appHost,
                  endpoint,
                  sessionToken,
                  foreground,
                  plugins,
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
        throw RuntimeError.configuration("app bundle resources are unavailable")
      }
      let storage = try Self.storageDirectory()
      handle = bundle.path.withCString { bundlePath in
        state.path.withCString { statePath in
          storage.path.withCString { storagePath in
            appHost.withCString { appHost in
              error.withUnsafeMutableBufferPointer { error in
                tokamak_runtime_start(
                  bundlePath,
                  statePath,
                  storagePath,
                  appHost,
                  foreground,
                  plugins,
                  error.baseAddress,
                  error.count
                )
              }
            }
          }
        }
      }
    }
    guard handle != nil else {
      throw RuntimeError.runtime(String(cString: error))
    }
    try? FileManager.default.removeItem(at: state.appendingPathComponent(startupErrorFile))
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
      tokamak_runtime_restore_gateway(handle, error.baseAddress, error.count)
    }
    guard port != 0 else {
      throw RuntimeError.runtime(String(cString: error))
    }
    return port
  }

  /// Delivers the event `name` with `event`, JSON-serialisable, to the
  /// Worker's listeners, blocking until they return or `timeout` passes, and
  /// returns their reply, parsed from JSON, or nil.
  func emit(_ name: String, event: Any, timeout: TimeInterval) throws -> Any? {
    guard JSONSerialization.isValidJSONObject([event]) else {
      throw RuntimeError.runtime("the \(name) event is not JSON")
    }
    let event = String(
      decoding: try JSONSerialization.data(withJSONObject: event, options: .fragmentsAllowed),
      as: UTF8.self
    )
    var reply = TokamakBytes()
    var error = [CChar](repeating: 0, count: 1024)
    let succeeded = name.withCString { name in
      event.withCString { event in
        error.withUnsafeMutableBufferPointer { error in
          tokamak_runtime_emit(
            handle,
            name,
            event,
            UInt64(max(timeout, 0) * 1000),
            &reply,
            error.baseAddress,
            error.count
          )
        }
      }
    }
    defer { tokamak_bytes_free(reply) }
    guard succeeded, let json = reply.contents else {
      throw RuntimeError.runtime(String(cString: error))
    }
    let value = try JSONSerialization.jsonObject(with: json, options: .fragmentsAllowed)
    return value is NSNull ? nil : value
  }

  /// What the app serves at `path`, blocking until it arrives or `timeout`
  /// passes.
  func fetch(_ path: String, timeout: TimeInterval) throws -> Data {
    var body = TokamakBytes()
    var error = [CChar](repeating: 0, count: 1024)
    let succeeded = path.withCString { path in
      error.withUnsafeMutableBufferPointer { error in
        tokamak_runtime_fetch(
          handle,
          path,
          UInt64(max(timeout, 0) * 1000),
          &body,
          error.baseAddress,
          error.count
        )
      }
    }
    defer { tokamak_bytes_free(body) }
    guard succeeded else {
      throw RuntimeError.runtime(String(cString: error))
    }
    return body.contents ?? Data()
  }

  /// Records whether the app is in the foreground, which the Worker's
  /// `getLifecycleStage()` reports from then on, and returns whether that
  /// changed.
  func setForeground(_ foreground: Bool) -> Bool {
    tokamak_runtime_set_foreground(handle, foreground)
  }

  /// Whether the app is in the foreground, as last recorded.
  var isForeground: Bool {
    tokamak_runtime_is_foreground(handle)
  }

  /// Passes a plugin's `result`, JSON-serialisable, to the Worker's call or
  /// subscription `id`.
  func reply(_ id: UInt64, result: [String: Any]) {
    guard let json = try? JSONSerialization.data(withJSONObject: result) else { return }
    String(decoding: json, as: UTF8.self).withCString { tokamak_runtime_reply(handle, id, $0) }
  }

  func serverAuthority(host: String) -> AuthenticationMaterial<Data> {
    var bytes = TokamakBytes()
    let decision = host.withCString {
      tokamak_runtime_server_authority(handle, $0, &bytes)
    }
    switch decision {
    case Int32(TOKAMAK_DECISION_DEFAULT):
      return .defaultHandling
    case Int32(TOKAMAK_DECISION_USE):
      defer { tokamak_bytes_free(bytes) }
      guard let authority = bytes.contents else { return .cancel }
      return .use(authority)
    default:
      return .cancel
    }
  }

  func clientIdentity(host: String, previousFailures: Int) -> AuthenticationMaterial<(Data, Data)> {
    var identity = TokamakIdentity()
    let decision = host.withCString {
      tokamak_runtime_client_identity(handle, $0, max(previousFailures, 0), &identity)
    }
    switch decision {
    case Int32(TOKAMAK_DECISION_DEFAULT):
      return .defaultHandling
    case Int32(TOKAMAK_DECISION_USE):
      defer { tokamak_identity_free(identity) }
      guard
        let certificate = identity.certificate.contents,
        let privateKey = identity.private_key.contents
      else {
        return .cancel
      }
      return .use((certificate, privateKey))
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

  /// Stores behind storage bindings, which device backups include.
  private static func storageDirectory() throws -> URL {
    let storage = try appDirectory(.applicationSupportDirectory)
      .appendingPathComponent("tokamak/storage", isDirectory: true)
    try FileManager.default.createDirectory(at: storage, withIntermediateDirectories: true)
    return storage
  }

  private static func stateDirectory() throws -> URL {
    let state = try appDirectory(.cachesDirectory)
    try FileManager.default.createDirectory(at: state, withIntermediateDirectories: true)
    return state
  }

  /// This app's folder in the user's `directory`.
  private static func appDirectory(_ directory: FileManager.SearchPathDirectory) throws -> URL {
    guard let identifier = Bundle.main.bundleIdentifier else {
      throw RuntimeError.configuration("CFBundleIdentifier is required")
    }
    return try FileManager.default.url(
      for: directory,
      in: .userDomainMask,
      appropriateFor: nil,
      create: true
    )
    .appendingPathComponent(identifier, isDirectory: true)
  }
}

extension TokamakBytes {
  /// A copy of the buffer, or nil when it is null.
  fileprivate var contents: Data? {
    data.map { Data(bytes: $0, count: len) }
  }
}
