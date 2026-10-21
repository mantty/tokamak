import Foundation

/// Native objects that plugins share by ID. Each belongs to a scope and is
/// kept until it is released or its scope ends.
final class TokamakResources {
  private let lock = NSLock()
  private var objects: [String: (object: Any, scope: String)] = [:]
  private var scopes: [String: Set<String>] = [:]

  /// Starts a scope and returns its ID.
  func open() -> String {
    let scope = UUID().uuidString
    lock.withLock { scopes[scope] = [] }
    return scope
  }

  /// Keeps `object` in `scope` and returns its ID. An ended scope keeps
  /// nothing, so the ID finds no object.
  func retain(_ object: Any, in scope: String) -> String {
    let id = UUID().uuidString
    lock.withLock {
      guard scopes[scope]?.insert(id) != nil else { return }
      objects[id] = (object, scope)
    }
    return id
  }

  /// Keeps `object` in a scope of its own, replies with its ID, and returns
  /// the release to return from `subscribe`.
  func retain(_ object: Any, reply: TokamakPluginReply) -> () -> Void {
    let scope = open()
    reply(.success(retain(object, in: scope)))
    return { [self] in end(scope) }
  }

  /// Releases the object retained as `id`, if it is still kept.
  func release(_ id: Any) {
    let removed = lock.withLock { () -> Any? in
      guard let id = id as? String, let entry = objects.removeValue(forKey: id) else { return nil }
      scopes[entry.scope]?.remove(id)
      return entry.object
    }
    drop(removed)
  }

  /// Releases every object `scope` keeps, and keeps nothing in it later.
  func end(_ scope: String) {
    let removed = lock.withLock { () -> [Any] in
      (scopes.removeValue(forKey: scope) ?? []).compactMap {
        objects.removeValue(forKey: $0)?.object
      }
    }
    drop(removed)
  }

  /// The object retained as `id`.
  func get<Object>(_ id: Any, as type: Object.Type) throws(TokamakPluginError) -> Object {
    guard let id = id as? String, let object = lock.withLock({ objects[id]?.object }) else {
      throw TokamakPluginError(name: "InvalidStateError", message: "The native reference is closed")
    }
    guard let typed = object as? Object else {
      throw TokamakPluginError(
        name: "TypeError", message: "The native reference is not \(Object.self)")
    }
    return typed
  }

  /// Lets go of released objects on the main thread, where UI and run loop
  /// objects expect to be deallocated.
  private func drop(_ objects: Any?) {
    guard let objects else { return }
    DispatchQueue.main.async { withExtendedLifetime(objects) {} }
  }
}
