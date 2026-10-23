import Foundation

/// A plugin's exported methods and events: their description, which `tok`
/// generates the TypeScript API from, and the Swift dispatch that serves them.
public struct Described {
  /// The description, as JSON.
  public let description: Data
  /// The Swift extension that routes the Worker's requests to the methods and
  /// encodes the events.
  public let dispatch: String
}

/// An exported method and the platforms that have it.
struct PlatformExport {
  let export: Export
  let platforms: [String]
}

/// A registered event and the platforms that declare its type.
struct PlatformEvent {
  let event: Event
  let platforms: [String]
}

/// What one platform's symbol graph resolves: each registration's method or event, if the
/// platform has it, and the types they use.
private struct Resolved {
  let platform: String
  let exports: [Export?]
  let events: [Event?]
  let types: [String: NamedType]
  let unions: [String: UnionCoding]
}

/// A union's coding and the platforms that declare it.
struct PlatformUnion {
  let coding: UnionCoding
  var platforms: [String]
}

/// Describes the methods and events the registration file `exports` lists,
/// resolved in each platform's symbol graph.
public func describe(
  exports: String, file: String, symbolGraphs: [(platform: String, graph: Data)]
) throws(DescribeError) -> Described {
  let registrations = try readRegistrations(exports, file: file)
  var resolved: [Resolved] = []
  for (platform, data) in symbolGraphs {
    let graph: SymbolGraph
    do {
      graph = try JSONDecoder().decode(SymbolGraph.self, from: data)
    } catch {
      throw DescribeError("the \(platform) symbol graph cannot be read: \(error)")
    }
    var resolver = try Resolver(graph, plugin: registrations.plugin)
    var exports: [Export?] = []
    for registration in registrations.methods {
      exports.append(try resolver.export(registration))
    }
    var events: [Event?] = []
    for registration in registrations.events {
      events.append(try resolver.event(registration))
    }
    resolved.append(
      Resolved(
        platform: platform, exports: exports, events: events,
        types: resolver.types.mapValues(\.type), unions: resolver.unions))
  }
  var methods: [PlatformExport] = []
  for (index, registration) in registrations.methods.enumerated() {
    let found = resolved.compactMap { entry in entry.exports[index].map { (entry.platform, $0) } }
    guard let (platform, export) = found.first else {
      throw DescribeError("\(registrations.plugin) has no method \(registration.name)")
    }
    if let (other, _) = found.first(where: { $0.1 != export }) {
      throw DescribeError(
        "\(registrations.plugin).\(export.title) differs between \(platform) and \(other)")
    }
    methods.append(PlatformExport(export: export, platforms: found.map(\.0)))
  }
  var events: [PlatformEvent] = []
  for (index, registration) in registrations.events.enumerated() {
    let found = resolved.compactMap { entry in entry.events[index].map { (entry.platform, $0) } }
    guard let (platform, event) = found.first else {
      throw DescribeError(
        "event \(registration.name): \(registration.type) is not a type the plugin declares")
    }
    if let (other, _) = found.first(where: { $0.1 != event }) {
      throw DescribeError("event \(registration.name) differs between \(platform) and \(other)")
    }
    events.append(PlatformEvent(event: event, platforms: found.map(\.0)))
  }
  var types: [String: NamedType] = [:]
  var unions: [String: PlatformUnion] = [:]
  for entry in resolved {
    for (name, type) in entry.types {
      if let existing = types[name], existing != type {
        throw DescribeError("\(name) differs between platforms")
      }
      types[name] = type
    }
    for (name, coding) in entry.unions {
      unions[name, default: PlatformUnion(coding: coding, platforms: [])].platforms.append(
        entry.platform)
    }
  }
  let platforms = symbolGraphs.map(\.platform)
  return Described(
    description: try encode(methods, events: events, types: types, platforms: platforms),
    dispatch: dispatch(
      plugin: registrations.plugin, methods: methods, events: events, platforms: platforms,
      types: types, unions: unions)
  )
}

private struct Description: Encodable {
  struct Method: Encodable {
    let name: String
    let kind: String
    let doc: String?
    let parameters: [Field]?
    let returns: DescribedType?
    let overflow: String?
    let platforms: [String]?
  }

  struct Event: Encodable {
    let name: String
    let doc: String?
    let type: DescribedType
    let reply: DescribedType?
    let platforms: [String]?
  }

  let methods: [Method]
  let events: [Event]?
  let types: [NamedType]
}

private func encode(
  _ methods: [PlatformExport], events: [PlatformEvent], types: [String: NamedType],
  platforms: [String]
) throws(DescribeError) -> Data {
  let description = Description(
    methods: methods.map { method in
      let export = method.export
      let registration = export.registration
      return Description.Method(
        name: registration.name,
        kind: registration.kind.rawValue,
        doc: registration.doc.isEmpty ? nil : registration.doc,
        parameters: export.parameters.isEmpty
          ? nil : export.parameters.map { Field(name: $0.name, type: $0.type) },
        returns: export.result,
        overflow: registration.dropsOldest ? "dropOldest" : nil,
        platforms: method.platforms == platforms ? nil : method.platforms
      )
    },
    events: events.isEmpty
      ? nil
      : events.map { event in
        let registration = event.event.registration
        return Description.Event(
          name: registration.name,
          doc: registration.doc.isEmpty ? nil : registration.doc,
          type: event.event.type,
          reply: event.event.reply,
          platforms: event.platforms == platforms ? nil : event.platforms
        )
      },
    types: types.values.sorted { $0.name < $1.name }
  )
  let encoder = JSONEncoder()
  encoder.outputFormatting = [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes]
  do {
    return try encoder.encode(description)
  } catch {
    throw DescribeError("the description cannot be encoded: \(error)")
  }
}
