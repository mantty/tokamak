import Foundation

/// Native objects that plugins share by ID, each kept while the subscription
/// that retained it is open.
final class TokamakResources {
  private let lock = NSLock()
  private var objects: [String: Any] = [:]

  /// Keeps `object`, replies with its ID, and returns the release to return
  /// from `subscribe`.
  func retain(_ object: Any, reply: TokamakPluginReply) -> () -> Void {
    let id = UUID().uuidString
    lock.withLock { objects[id] = object }
    reply(.success(id))
    return { [self] in _ = lock.withLock { objects.removeValue(forKey: id) } }
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
}
