import Foundation

/// A type as the description names it: a JSON value, or bytes, a body or a
/// native reference, which the transport carries apart from JSON.
indirect enum DescribedType: Equatable, Encodable, Sendable {
  case string, number, boolean, void, bytes, body, reference, json
  case optional(DescribedType)
  case array(DescribedType)
  case record(DescribedType)
  case named(String)

  private enum Key: String, CodingKey {
    case optional, array, record, named
  }

  var isOptional: Bool {
    if case .optional = self { true } else { false }
  }

  func encode(to encoder: any Encoder) throws {
    switch self {
    case .optional(let wrapped): try encode(wrapped, as: .optional, to: encoder)
    case .array(let element): try encode(element, as: .array, to: encoder)
    case .record(let value): try encode(value, as: .record, to: encoder)
    case .named(let name):
      var container = encoder.container(keyedBy: Key.self)
      try container.encode(name, forKey: .named)
    default:
      var container = encoder.singleValueContainer()
      try container.encode("\(self)")
    }
  }

  private func encode(_ inner: DescribedType, as key: Key, to encoder: any Encoder) throws {
    var container = encoder.container(keyedBy: Key.self)
    try container.encode(inner, forKey: key)
  }
}

/// A type as declaration fragments spell it, before it is described.
indirect enum SpelledType: Equatable {
  /// A named type, with its symbol identifier and generic arguments.
  case name(String, identifier: String?, arguments: [SpelledType])
  case array(SpelledType)
  case dictionary(key: SpelledType, value: SpelledType)
  case optional(SpelledType)
  case tuple([SpelledType])
}

extension SpelledType {
  /// The type as swift-format writes it, such as `[String: Region]?`.
  var source: String {
    spelled { name, _ in name }
  }

  /// The type as swift-format writes it, with each name as `name` gives it from the name and its
  /// symbol identifier.
  func spelled(_ name: (String, String?) -> String) -> String {
    switch self {
    case .name(let spelling, let identifier, let arguments):
      let named = name(spelling, identifier)
      return arguments.isEmpty
        ? named : "\(named)<\(arguments.map { $0.spelled(name) }.joined(separator: ", "))>"
    case .array(let element): return "[\(element.spelled(name))]"
    case .dictionary(let key, let value): return "[\(key.spelled(name)): \(value.spelled(name))]"
    case .optional(let wrapped): return "\(wrapped.spelled(name))?"
    case .tuple(let elements):
      return "(\(elements.map { $0.spelled(name) }.joined(separator: ", ")))"
    }
  }
}

/// Reads the type that `fragments` spell, such as `[String : Region]?`.
func spelledType(_ fragments: [SymbolGraph.Fragment]) throws(DescribeError) -> SpelledType {
  try parse(tokens(fragments), spelling: fragments.map(\.spelling).joined())
}

/// Reads the type that `source` spells, such as `[String: Region]`, with each
/// name's symbol identifier from `identifier`, or nil when a name has none.
func spelledType(
  _ source: String, identifier: (String) -> String?
) throws(DescribeError) -> SpelledType? {
  let tokens = textTokens(source).map { token in
    guard case .keyword(let word) = token, word != "any" else { return token }
    return .name(word, identifier: nil)
  }
  return try parse(tokens, spelling: source).resolved(identifier)
}

private func parse(_ tokens: [Token], spelling: String) throws(DescribeError) -> SpelledType {
  var parser = TypeParser(tokens: tokens, spelling: spelling)
  let type = try parser.type()
  guard parser.tokens.isEmpty else { throw parser.unsupported() }
  return type
}

extension SpelledType {
  /// The type with each name's identifier from `identifier`, or nil when a name has none.
  fileprivate func resolved(_ identifier: (String) -> String?) -> SpelledType? {
    switch self {
    case .name(let name, _, let arguments):
      guard let resolved = identifier(name), let arguments = arguments.resolved(identifier) else {
        return nil
      }
      return .name(name, identifier: resolved, arguments: arguments)
    case .array(let element): return element.resolved(identifier).map(SpelledType.array)
    case .dictionary(let key, let value):
      guard let key = key.resolved(identifier), let value = value.resolved(identifier) else {
        return nil
      }
      return .dictionary(key: key, value: value)
    case .optional(let wrapped): return wrapped.resolved(identifier).map(SpelledType.optional)
    case .tuple(let elements): return elements.resolved(identifier).map(SpelledType.tuple)
    }
  }
}

extension [SpelledType] {
  fileprivate func resolved(_ identifier: (String) -> String?) -> [SpelledType]? {
    var types: [SpelledType] = []
    for type in self {
      guard let resolved = type.resolved(identifier) else { return nil }
      types.append(resolved)
    }
    return types
  }
}

private enum Token: Equatable {
  case name(String, identifier: String?)
  case punctuation(Character)
  case keyword(String)
}

private func tokens(_ fragments: [SymbolGraph.Fragment]) -> [Token] {
  fragments.flatMap { fragment -> [Token] in
    switch fragment.kind {
    case "typeIdentifier": [.name(fragment.spelling, identifier: fragment.preciseIdentifier)]
    case "text": textTokens(fragment.spelling)
    default: [.keyword(fragment.spelling)]
    }
  }
}

/// The words, such as `any`, and punctuation of text between type names.
private func textTokens(_ text: String) -> [Token] {
  var tokens: [Token] = []
  var word = ""
  for character in text + " " {
    if character.isLetter || character.isNumber || character == "_" {
      word.append(character)
      continue
    }
    if !word.isEmpty { tokens.append(.keyword(word)) }
    word = ""
    if !character.isWhitespace { tokens.append(.punctuation(character)) }
  }
  return tokens
}

private struct TypeParser {
  var tokens: [Token]
  let spelling: String

  mutating func type() throws(DescribeError) -> SpelledType {
    var type = try primary()
    while tokens.first == .punctuation("?") {
      tokens.removeFirst()
      type = .optional(type)
    }
    return type
  }

  private mutating func primary() throws(DescribeError) -> SpelledType {
    switch tokens.first {
    case .punctuation("["):
      tokens.removeFirst()
      let element = try type()
      if tokens.first == .punctuation(":") {
        tokens.removeFirst()
        let value = try type()
        try expect("]")
        return .dictionary(key: element, value: value)
      }
      try expect("]")
      return .array(element)
    case .punctuation("("):
      tokens.removeFirst()
      return .tuple(try list(until: ")"))
    case .keyword("any"):
      tokens.removeFirst()
      return try primary()
    case .name(let name, var identifier):
      tokens.removeFirst()
      // A member type, such as `Outer.Inner`, is the last name.
      var spelled = name
      while tokens.first == .punctuation("."),
        case .name(let member, let memberIdentifier)? = tokens.dropFirst().first
      {
        tokens.removeFirst(2)
        spelled += ".\(member)"
        identifier = memberIdentifier
      }
      guard tokens.first == .punctuation("<") else {
        return .name(spelled, identifier: identifier, arguments: [])
      }
      tokens.removeFirst()
      return .name(spelled, identifier: identifier, arguments: try list(until: ">"))
    default:
      throw unsupported()
    }
  }

  private mutating func list(until end: Character) throws(DescribeError) -> [SpelledType] {
    var types: [SpelledType] = []
    while tokens.first != .punctuation(end) {
      if !types.isEmpty { try expect(",") }
      types.append(try type())
    }
    tokens.removeFirst()
    return types
  }

  private mutating func expect(_ character: Character) throws(DescribeError) {
    guard tokens.first == .punctuation(character) else { throw unsupported() }
    tokens.removeFirst()
  }

  func unsupported() -> DescribeError {
    DescribeError(
      "\(spelling.trimmingCharacters(in: .whitespaces)) is not a type the Worker can pass")
  }
}
