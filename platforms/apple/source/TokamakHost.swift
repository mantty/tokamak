import Foundation
import TokamakRuntime

private let startupErrorFile = "startup-error.log"

/// The tokamak runtime and native plugins of this process. Both outlive any
/// WebView, so plugins receive app events before a page loads and a Worker
/// event can run while no page is loaded.
final class TokamakHost {
  private(set) var plugins: [any TokamakPlugin] = []
  private var runtime: Result<RuntimeHandle, Error>?
  private var waiting: [(Result<RuntimeHandle, Error>) -> Void] = []

  /// Creates the plugins and starts the runtime in the background. Call it
  /// before launch finishes.
  init() {
    plugins = tokamakPlugins(host: self)
    DispatchQueue.global(qos: .userInitiated).async {
      let result = Result { try RuntimeHandle() }
      DispatchQueue.main.async { self.started(result) }
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

  /// Runs the Worker's `event` handler with a JSON-serialisable `payload`,
  /// including work it passes to `ctx.waitUntil`. Call it on the main thread;
  /// `completion` runs on the main thread.
  func dispatch(
    event: String,
    payload: Any,
    completion: @escaping (Result<TokamakEventOutcome, Error>) -> Void
  ) {
    whenStarted { result in
      switch result {
      case .failure(let error):
        completion(.failure(error))
      case .success(let runtime):
        DispatchQueue.global(qos: .userInitiated).async {
          let outcome = Result { try runtime.dispatch(event: event, payload: payload) }
          DispatchQueue.main.async { completion(outcome) }
        }
      }
    }
  }

  func didRegisterForRemoteNotifications(deviceToken: Data) {
    for plugin in plugins {
      plugin.didRegisterForRemoteNotifications(deviceToken: deviceToken)
    }
  }

  func didFailToRegisterForRemoteNotifications(error: Error) {
    for plugin in plugins {
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
    for plugin in plugins {
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

/// What a Worker event handler did.
enum TokamakEventOutcome {
  /// The handler's result, decoded from JSON; `NSNull` when it returned nothing.
  case handled(Any)
  /// The Worker has no handler for the event.
  case unhandled
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

  init() throws {
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
      handle = bundle.path.withCString { bundlePath in
        state.path.withCString { statePath in
          appHost.withCString { appHost in
            error.withUnsafeMutableBufferPointer { error in
              tokamak_runtime_start(
                bundlePath,
                statePath,
                appHost,
                error.baseAddress,
                error.count
              )
            }
          }
        }
      }
    }
    guard handle != nil else {
      throw RuntimeError.runtime(String(cString: error))
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
      throw RuntimeError.runtime(String(cString: error))
    }
    return port
  }

  /// Runs the Worker's `event` handler, blocking until it settles.
  func dispatch(event: String, payload: Any) throws -> TokamakEventOutcome {
    let payload = String(
      decoding: try JSONSerialization.data(withJSONObject: payload, options: .fragmentsAllowed),
      as: UTF8.self
    )
    var result = TokamakBytes(data: nil, len: 0)
    var error = [CChar](repeating: 0, count: 1024)
    let status = event.withCString { event in
      payload.withCString { payload in
        error.withUnsafeMutableBufferPointer { error in
          tokamak_runtime_dispatch(handle, event, payload, &result, error.baseAddress, error.count)
        }
      }
    }
    defer { tokamak_bytes_free(result) }
    switch status {
    case Int32(TOKAMAK_DISPATCH_HANDLED):
      guard let bytes = result.data else {
        throw RuntimeError.runtime("the Worker event result is missing")
      }
      return .handled(
        try JSONSerialization.jsonObject(
          with: Data(bytes: bytes, count: result.len),
          options: .fragmentsAllowed
        ))
    case Int32(TOKAMAK_DISPATCH_UNHANDLED):
      return .unhandled
    default:
      throw RuntimeError.runtime(String(cString: error))
    }
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
      throw RuntimeError.configuration("CFBundleIdentifier is required")
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
