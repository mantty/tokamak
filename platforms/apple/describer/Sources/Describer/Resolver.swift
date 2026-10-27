import Foundation
import SwiftParser
import SwiftSyntax

/// An exported method as one platform's compiler declares it.
struct Export: Equatable {
  let registration: Registration
  /// The method's name with its argument labels, such as `addRegion(_:replace:)`.
  let title: String
  let parameters: [Parameter]
  /// A call's result or a stream's values; a resource has none.
  let result: DescribedType?
  let throwing: Bool
  /// Whether a call suspends: the method is async, or isolated to a global actor.
  let awaited: Bool
}

/// A registered event as one platform's compiler declares its types.
struct Event: Equatable {
  let registration: EventRegistration
  let type: DescribedType
  let reply: DescribedType?
}

struct Parameter: Equatable {
  let label: String
  let name: String
  /// The type as the method's declaration spells it.
  let spelling: String
  let type: DescribedType
}

/// A struct with its fields, a string enum with its values, or a union with the forms its values
/// take and the field that tags them, if any.
struct NamedType: Equatable, Encodable {
  let name: String
  var fields: [Field]?
  var values: [String]?
  var alternatives: [Alternative]?
  var tag: String?
}

/// A union's form: a case's name, and its associated value's type, if it has one.
struct Alternative: Equatable, Encodable {
  let name: String
  let type: DescribedType?
}

/// A union's Swift type and the spelling of each case's associated value, which its generated
/// coding names.
struct UnionCoding: Equatable {
  /// The type's path, such as `TokamakNotificationsPlugin.Sound`.
  let path: String
  /// Each case's associated value, spelled with the paths of the types the plugin declares, by
  /// case name.
  let payloads: [String: String]
}

struct Field: Equatable, Encodable {
  let name: String
  let type: DescribedType
}

/// Each standard type's name and JSON type, by symbol identifier.
private let standardTypes: [String: (name: String, type: DescribedType)] = [
  "s:SS": ("String", .string), "s:Sb": ("Bool", .boolean), "s:s4Voida": ("Void", .void),
  "s:10Foundation4DataV": ("Data", .bytes), "s:Si": ("Int", .number),
  "s:s4Int8V": ("Int8", .number), "s:s5Int16V": ("Int16", .number),
  "s:s5Int32V": ("Int32", .number), "s:s5Int64V": ("Int64", .number), "s:Su": ("UInt", .number),
  "s:s5UInt8V": ("UInt8", .number), "s:s6UInt16V": ("UInt16", .number),
  "s:s6UInt32V": ("UInt32", .number), "s:s6UInt64V": ("UInt64", .number),
  "s:Sd": ("Double", .number), "s:Sf": ("Float", .number),
  "s:14CoreFoundation7CGFloatV": ("CGFloat", .number),
]

/// Resolves registrations, and the types they use, in one platform's symbol graph.
struct Resolver {
  private let index: SymbolIndex
  private let plugin: SymbolGraph.Symbol
  private var sources = Sources()
  /// The described types by name, with the identifiers of their symbols.
  private(set) var types: [String: (identifier: String, type: NamedType)] = [:]
  /// The described unions' codings, by name.
  private(set) var unions: [String: UnionCoding] = [:]

  init(_ graph: SymbolGraph, plugin: String) throws(DescribeError) {
    index = SymbolIndex(graph)
    guard let symbol = index.type(at: plugin.split(separator: ".").map(String.init)) else {
      throw DescribeError("\(plugin) is not a type the plugin declares")
    }
    self.plugin = symbol
  }

  /// The method `registration` names, or nil when this platform has none.
  mutating func export(_ registration: Registration) throws(DescribeError) -> Export? {
    // The registration file sees the methods the plugin does not keep to its own files.
    let candidates = index.members(of: plugin).filter { method in
      method.kind.identifier == "swift.method"
        && !["private", "fileprivate"].contains(method.accessLevel)
        && method.name.hasPrefix("\(registration.name)(")
        && registration.labels.map { method.name == title(registration.name, $0) } ?? true
    }
    guard let method = candidates.first else { return nil }
    guard candidates.count == 1 else {
      throw DescribeError(
        "\(plugin.name).\(registration.name) is overloaded; name it with its labels, as in \(method.name)"
      )
    }
    do {
      return try export(registration, method)
    } catch {
      throw DescribeError("\(plugin.name).\(method.name): \(error)")
    }
  }

  private mutating func export(
    _ registration: Registration, _ method: SymbolGraph.Symbol
  ) throws(DescribeError) -> Export {
    let labels = method.name.dropFirst(registration.name.count + 1).dropLast().split(separator: ":")
    var parameters: [Parameter] = []
    for (label, parameter) in zip(labels, method.functionSignature?.parameters ?? []) {
      let spelled = try spelledType(typeFragments(parameter.declarationFragments))
      parameters.append(
        Parameter(
          label: String(label), name: parameter.name, spelling: spelled.source,
          type: try describe(spelled)))
    }
    let returns = try spelledType(method.functionSignature?.returns ?? [])
    let result: DescribedType? =
      switch registration.kind {
      case .call: try describe(returns)
      case .stream: try describe(try element(of: returns))
      case .resource: nil
      }
    return Export(
      registration: registration,
      title: method.name,
      parameters: parameters,
      result: result,
      throwing: method.has("throws"),
      awaited: method.has("async")
        || method.declarationFragments.contains {
          $0.kind == "attribute" && $0.preciseIdentifier != nil
        }
    )
  }

  /// The event `registration` lists, or nil when this platform lacks its type.
  mutating func event(_ registration: EventRegistration) throws(DescribeError) -> Event? {
    guard let type = try spelledType(registration.type, identifier: identifier(named:)) else {
      return nil
    }
    var reply: SpelledType?
    if let spelling = registration.reply {
      reply = try spelledType(spelling, identifier: identifier(named:))
      guard reply != nil else {
        throw DescribeError(
          "event \(registration.name): \(spelling) is not a type the plugin declares")
      }
    }
    do {
      let described = try describe(type)
      return Event(
        registration: registration, type: described, reply: try reply.map { try describe($0) })
    } catch {
      throw DescribeError("event \(registration.name): \(error)")
    }
  }

  /// The identifier of the type `name` as the plugin's extensions see it: a
  /// type the plugin nests, a top-level type, or a standard one.
  private func identifier(named name: String) -> String? {
    let path = name.split(separator: ".").map(String.init)
    let type = index.type(at: plugin.pathComponents + path) ?? index.type(at: path)
    return type?.identifier.precise ?? standardTypes.first { $0.value.name == name }?.key
  }

  /// The values of the stream `type`: `AsyncStream` or `AsyncThrowingStream`.
  private func element(of type: SpelledType) throws(DescribeError) -> SpelledType {
    guard case .name(_, let identifier, let arguments) = type,
      ["s:ScS", "s:Scs"].contains(identifier), let element = arguments.first
    else {
      throw DescribeError("a stream returns AsyncStream or AsyncThrowingStream")
    }
    return element
  }

  private mutating func describe(_ type: SpelledType) throws(DescribeError) -> DescribedType {
    switch type {
    case .optional(let wrapped): return .optional(try describe(wrapped))
    case .array(let element): return .array(try describe(element))
    case .dictionary(let key, let value): return try record(key, value)
    case .tuple([]): return .void
    case .tuple: throw DescribeError("tuples are not types the Worker can pass")
    case .name(let spelling, let identifier, let arguments):
      return try describe(spelling, identifier ?? "", arguments)
    }
  }

  private mutating func describe(
    _ spelling: String, _ identifier: String, _ arguments: [SpelledType]
  ) throws(DescribeError) -> DescribedType {
    if let standard = standardTypes[identifier] { return standard.type }
    switch (identifier, arguments.count) {
    case ("s:Sq", 1): return .optional(try describe(arguments[0]))
    case ("s:Sa", 1): return .array(try describe(arguments[0]))
    case ("s:SD", 2): return try record(arguments[0], arguments[1])
    default: break
    }
    guard let symbol = index[identifier] else {
      throw DescribeError("\(spelling) is not a type the Worker can pass")
    }
    switch symbol.name {
    case "TokamakPluginBody": return .body
    case "TokamakReference": return .reference
    case "TokamakJSON": return .json
    default: return try named(symbol)
    }
  }

  private mutating func record(
    _ key: SpelledType, _ value: SpelledType
  ) throws(DescribeError) -> DescribedType {
    guard try describe(key) == .string else {
      throw DescribeError("dictionaries the Worker passes have String keys")
    }
    return .record(try describe(value))
  }

  /// Describes the struct or string enum `symbol` once, by name.
  private mutating func named(_ symbol: SymbolGraph.Symbol) throws(DescribeError) -> DescribedType {
    let identifier = symbol.identifier.precise
    if let existing = types[symbol.name] {
      guard existing.identifier == identifier else {
        throw DescribeError("two types the Worker passes are named \(symbol.name)")
      }
      return .named(symbol.name)
    }
    // Recorded first, so a type that contains itself refers to it by name.
    types[symbol.name] = (identifier, NamedType(name: symbol.name))
    let type: NamedType
    do {
      type =
        switch symbol.kind.identifier {
        case "swift.struct": NamedType(name: symbol.name, fields: try fields(of: symbol))
        case "swift.enum" where index.conforms(symbol, to: "TokamakUnion"): try union(symbol)
        case "swift.enum": NamedType(name: symbol.name, values: try values(of: symbol))
        default:
          throw DescribeError(
            "only structs, string enums and TokamakUnion enums are types the Worker can pass")
        }
    } catch {
      throw DescribeError("\(symbol.name): \(error)")
    }
    types[symbol.name] = (identifier, type)
    return .named(symbol.name)
  }

  /// The stored properties of a `Codable` struct, which its synthesized
  /// conformance encodes.
  private mutating func fields(of symbol: SymbolGraph.Symbol) throws(DescribeError) -> [Field] {
    if symbol.declarationFragments.contains(where: { $0.spelling.contains("<") }) {
      throw DescribeError("generic structs are not types the Worker can pass")
    }
    let members = index.members(of: symbol)
    if members.contains(where: { $0.name == "CodingKeys" }) {
      throw DescribeError("its coding keys rename fields the generated API names")
    }
    var fields: [Field] = []
    for property in members
    where property.kind.identifier == "swift.property" && !property.has("get") {
      let type = try describe(try spelledType(typeFragments(property.declarationFragments)))
      // Codable encodes a `let` with an initial value but never decodes it.
      let decoded = try !sources.isInitialized(property)
      fields.append(
        Field(name: property.name, type: decoded || type.isOptional ? type : .optional(type)))
    }
    return fields
  }

  /// The union `symbol`: each case's name and associated value, if any, and the tag of a
  /// `TokamakTaggedUnion`.
  private mutating func union(_ symbol: SymbolGraph.Symbol) throws(DescribeError) -> NamedType {
    var alternatives: [Alternative] = []
    var payloads: [String: String] = [:]
    let members = index.members(of: symbol)
    for item in members where item.kind.identifier == "swift.enum.case" {
      // A case with an associated value is named with its labels, as in `volume(_:)`.
      let name = String(item.name.prefix { $0 != "(" })
      let payload = try associatedValue(of: item)
      let type = try payload.map { payload throws(DescribeError) in try describe(payload) }
      alternatives.append(Alternative(name: name, type: type))
      // The coding is an extension of the union, from which a type its enclosing type declares
      // is found only by its path.
      payloads[name] = payload?.spelled { name, identifier in
        identifier.flatMap { index[$0]?.pathComponents.joined(separator: ".") } ?? name
      }
    }
    var tag: String?
    if index.conforms(symbol, to: "TokamakTaggedUnion") {
      guard let member = members.first(where: { $0.name == "tag" }),
        let literal = try sources.literal(of: member)
      else {
        throw DescribeError("a TokamakTaggedUnion declares its tag as static let tag = \"...\"")
      }
      tag = literal
    }
    unions[symbol.name] = UnionCoding(
      path: symbol.pathComponents.joined(separator: "."), payloads: payloads)
    // By name, as Kotlin's reflection lists no declaration order.
    return NamedType(
      name: symbol.name, alternatives: alternatives.sorted { $0.name < $1.name }, tag: tag)
  }

  /// The type of the enum case `symbol`'s one unlabeled associated value, or nil when it has none.
  private func associatedValue(of symbol: SymbolGraph.Symbol) throws(DescribeError) -> SpelledType?
  {
    let fragments = symbol.declarationFragments
    guard let name = fragments.firstIndex(where: { $0.kind == "identifier" }),
      name + 1 < fragments.count
    else {
      return nil
    }
    let value = Array(fragments[(name + 1)...])
    if value.contains(where: { $0.kind == "externalParam" || $0.kind == "internalParam" }) {
      throw DescribeError("\(symbol.name) has an unlabeled associated value")
    }
    guard case .tuple(let elements) = try spelledType(value), elements.count == 1 else {
      throw DescribeError("\(symbol.name) has one associated value")
    }
    return elements[0]
  }

  /// The raw values of a `String` enum's cases.
  private mutating func values(of symbol: SymbolGraph.Symbol) throws(DescribeError) -> [String] {
    let members = index.members(of: symbol)
    let rawValue = members.first { $0.name == "init(rawValue:)" }?.functionSignature?.parameters?
      .first
    guard let rawValue,
      typeFragments(rawValue.declarationFragments).compactMap(\.preciseIdentifier) == ["s:SS"]
    else {
      throw DescribeError("enums the Worker passes have String raw values")
    }
    var values: [String] = []
    for item in members where item.kind.identifier == "swift.enum.case" {
      guard !item.declarationFragments.contains(where: { $0.spelling.contains("(") }) else {
        throw DescribeError("\(item.name) has associated values")
      }
      values.append(try sources.rawValue(of: item) ?? item.name)
    }
    return values
  }
}

/// `name(label:...)`, the title of a method with `labels`.
private func title(_ name: String, _ labels: [String]) -> String {
  "\(name)(\(labels.map { "\($0):" }.joined()))"
}

/// The fragments that spell the type of a declaration such as `let id: String`
/// or `region: Region = .home`.
private func typeFragments(_ fragments: [SymbolGraph.Fragment]) -> [SymbolGraph.Fragment] {
  guard
    let colon = fragments.firstIndex(where: { $0.kind == "text" && $0.spelling.contains(":") })
  else {
    return fragments
  }
  let rest = String(fragments[colon].spelling.drop { $0 != ":" }.dropFirst())
  var type: [SymbolGraph.Fragment] = []
  for fragment in [text(rest)] + fragments[(colon + 1)...] {
    if fragment.kind == "text", let equals = fragment.spelling.firstIndex(of: "=") {
      type.append(text(String(fragment.spelling[..<equals])))
      break
    }
    type.append(fragment)
  }
  return type
}

private func text(_ spelling: String) -> SymbolGraph.Fragment {
  SymbolGraph.Fragment(kind: "text", spelling: spelling, preciseIdentifier: nil)
}

/// The raw values the plugin's sources give enum cases, which symbol graphs
/// leave out, parsed once per file.
private struct Sources {
  /// What the describer reads from one source file, by line and column.
  private struct File {
    /// Each enum case, with its raw value.
    var rawValues: [String: String?] = [:]
    /// The `let` properties with an initial value, which `Decodable` never decodes, with the
    /// value when it is a string literal.
    var initialized: [String: String?] = [:]
  }

  private var files: [String: File] = [:]

  /// The raw value the enum case `symbol` declares, or nil for its name.
  mutating func rawValue(of symbol: SymbolGraph.Symbol) throws(DescribeError) -> String? {
    let (file, key) = try declaration(of: symbol)
    guard let value = file.rawValues[key] else {
      throw DescribeError("\(key): \(symbol.name) is not an enum case")
    }
    return value
  }

  /// Whether the property `symbol` is a `let` with an initial value.
  mutating func isInitialized(_ symbol: SymbolGraph.Symbol) throws(DescribeError) -> Bool {
    let (file, key) = try declaration(of: symbol)
    return file.initialized[key] != nil
  }

  /// The string literal that initializes the `let` property `symbol`, or nil when none does.
  mutating func literal(of symbol: SymbolGraph.Symbol) throws(DescribeError) -> String? {
    let (file, key) = try declaration(of: symbol)
    return file.initialized[key] ?? nil
  }

  /// The file that declares `symbol`, and the declaration's line and column in it.
  private mutating func declaration(
    of symbol: SymbolGraph.Symbol
  ) throws(DescribeError) -> (File, String) {
    guard let location = symbol.location, let path = URL(string: location.uri)?.path else {
      throw DescribeError("\(symbol.name) has no source location")
    }
    if files[path] == nil {
      files[path] = try Self.read(path)
    }
    return (files[path]!, "\(location.position.line + 1):\(location.position.character + 1)")
  }

  private static func read(_ path: String) throws(DescribeError) -> File {
    guard let source = try? String(contentsOfFile: path, encoding: .utf8) else {
      throw DescribeError("cannot read \(path)")
    }
    let tree = Parser.parse(source: source)
    let converter = SourceLocationConverter(fileName: path, tree: tree)
    let declarations = Declarations(viewMode: .sourceAccurate)
    declarations.walk(tree)
    var file = File()
    for element in declarations.cases {
      let location = element.name.startLocation(converter: converter)
      let literal = element.rawValue?.value.as(StringLiteralExprSyntax.self)?
        .representedLiteralValue
      if element.rawValue != nil, literal == nil {
        throw DescribeError("\(path):\(location.line): raw values are string literals")
      }
      file.rawValues["\(location.line):\(location.column)"] = .some(literal)
    }
    for binding in declarations.initialized {
      let location = binding.pattern.startLocation(converter: converter)
      file.initialized["\(location.line):\(location.column)"] = .some(
        binding.initializer?.value.as(StringLiteralExprSyntax.self)?.representedLiteralValue)
    }
    return file
  }
}

/// The enum cases and initialized `let` bindings of a file.
private final class Declarations: SyntaxVisitor {
  private(set) var cases: [EnumCaseElementSyntax] = []
  private(set) var initialized: [PatternBindingSyntax] = []

  override func visit(_ node: EnumCaseElementSyntax) -> SyntaxVisitorContinueKind {
    cases.append(node)
    return .skipChildren
  }

  override func visit(_ node: VariableDeclSyntax) -> SyntaxVisitorContinueKind {
    if node.bindingSpecifier.tokenKind == .keyword(.let) {
      initialized += node.bindings.filter { $0.initializer != nil }
    }
    return .skipChildren
  }
}
