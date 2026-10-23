import Foundation

/// The symbols the compiler emits for a module, as `-emit-symbol-graph`
/// writes them.
struct SymbolGraph: Decodable {
  struct Symbol: Decodable {
    struct Kind: Decodable {
      let identifier: String
    }

    struct Identifier: Decodable {
      let precise: String
    }

    struct Signature: Decodable {
      struct Parameter: Decodable {
        let name: String
        let declarationFragments: [Fragment]
      }

      let parameters: [Parameter]?
      let returns: [Fragment]?
    }

    struct Location: Decodable {
      struct Position: Decodable {
        let line: Int
        let character: Int
      }

      let uri: String
      let position: Position
    }

    let kind: Kind
    let identifier: Identifier
    let accessLevel: String
    let pathComponents: [String]
    let declarationFragments: [Fragment]
    let functionSignature: Signature?
    let location: Location?

    var name: String { pathComponents.last ?? "" }

    /// Whether the declaration has `keyword`, such as `async` or `throws`.
    func has(_ keyword: String) -> Bool {
      declarationFragments.contains { $0.kind == "keyword" && $0.spelling == keyword }
    }
  }

  struct Fragment: Decodable, Equatable {
    let kind: String
    let spelling: String
    let preciseIdentifier: String?
  }

  struct Relationship: Decodable {
    let kind: String
    let source: String
    let target: String
  }

  let symbols: [Symbol]
  let relationships: [Relationship]
}

/// A symbol graph's symbols by identifier, and their members in source order.
struct SymbolIndex {
  private let symbols: [String: SymbolGraph.Symbol]
  private let members: [String: [SymbolGraph.Symbol]]
  /// The identifiers of the protocols each type conforms to, by the type's identifier.
  private let conformances: [String: [String]]

  init(_ graph: SymbolGraph) {
    symbols = Dictionary(graph.symbols.map { ($0.identifier.precise, $0) }) { first, _ in first }
    conformances = Dictionary(
      grouping: graph.relationships.filter { $0.kind == "conformsTo" }, by: \.source
    ).mapValues { $0.map(\.target) }
    let symbols = symbols
    members = Dictionary(
      grouping: graph.relationships.filter { $0.kind == "memberOf" }.compactMap { relationship in
        symbols[relationship.source].map { (relationship.target, $0) }
      },
      by: \.0
    ).mapValues { pairs in
      pairs.map(\.1).sorted {
        ($0.location?.position.line ?? 0, $0.location?.position.character ?? 0)
          < ($1.location?.position.line ?? 0, $1.location?.position.character ?? 0)
      }
    }
  }

  subscript(identifier: String) -> SymbolGraph.Symbol? { symbols[identifier] }

  /// The type at `path`, such as `["TokamakLocationPlugin"]`.
  func type(at path: [String]) -> SymbolGraph.Symbol? {
    let kinds = ["swift.class", "swift.struct", "swift.enum", "swift.actor"]
    return symbols.values.first { $0.pathComponents == path && kinds.contains($0.kind.identifier) }
  }

  func members(of symbol: SymbolGraph.Symbol) -> [SymbolGraph.Symbol] {
    members[symbol.identifier.precise] ?? []
  }

  /// Whether `symbol` conforms to the protocol named `name`, which the module declares.
  func conforms(_ symbol: SymbolGraph.Symbol, to name: String) -> Bool {
    (conformances[symbol.identifier.precise] ?? []).contains { symbols[$0]?.name == name }
  }
}
