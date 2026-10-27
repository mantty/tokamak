import Foundation

/// Native objects that plugins share by ID. Each is kept until the
/// subscription that retained it ends.
final class TokamakResources {
  private let lock = NSLock()
  private var objects: [String: Any] = [:]

  /// Keeps `object`, replies with its ID, and returns the release to return
  /// from `subscribe`.
  func retain(_ object: Any, reply: TokamakPluginBytesReply) -> () -> Void {
    let id = UUID().uuidString
    lock.withLock { objects[id] = object }
    reply(TokamakPluginResult(encodePluginValue(id), done: false))
    return { [self] in drop(lock.withLock { objects.removeValue(forKey: id) }) }
  }

  /// The object retained as `id`.
  func get<Object>(_ id: Any, as type: Object.Type) throws(TokamakPluginError) -> Object {
    guard let id = id as? String, let object = lock.withLock({ objects[id] }) else {
      throw TokamakPluginError(name: "InvalidStateError", message: "The native reference is closed")
    }
    guard let typed = object as? Object else {
      throw TokamakPluginError(
        name: "TypeError", message: "The native reference is not \(Object.self)")
    }
    return typed
  }

  /// Lets go of a released object on the main thread, where UI and run loop
  /// objects expect to be deallocated.
  private func drop(_ object: Any?) {
    guard let object else { return }
    DispatchQueue.main.async { withExtendedLifetime(object) {} }
  }
}
