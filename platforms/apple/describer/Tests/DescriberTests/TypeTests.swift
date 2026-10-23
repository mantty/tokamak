import Testing

@testable import Describer

private func fragments(_ parts: [(String, String?)]) -> [SymbolGraph.Fragment] {
  parts.map { spelling, identifier in
    SymbolGraph.Fragment(
      kind: identifier == nil ? "text" : "typeIdentifier", spelling: spelling,
      preciseIdentifier: identifier)
  }
}

@Test func readsSugaredAndGenericTypes() throws {
  let dictionary = try spelledType(
    fragments([("[", nil), ("String", "s:SS"), (" : [", nil), ("Int", "s:Si"), ("]]?", nil)]))
  #expect(
    dictionary
      == .optional(
        .dictionary(
          key: .name("String", identifier: "s:SS", arguments: []),
          value: .array(.name("Int", identifier: "s:Si", arguments: [])))))
  #expect(dictionary.source == "[String: [Int]]?")

  let stream = try spelledType(
    fragments([
      ("AsyncThrowingStream", "s:Scs"), ("<", nil), ("Region", "s:4Demo6RegionV"), (", any ", nil),
      ("Error", "s:s5ErrorP"), (">", nil),
    ]))
  #expect(
    stream
      == .name(
        "AsyncThrowingStream", identifier: "s:Scs",
        arguments: [
          .name("Region", identifier: "s:4Demo6RegionV", arguments: []),
          .name("Error", identifier: "s:s5ErrorP", arguments: []),
        ]))
  #expect(stream.source == "AsyncThrowingStream<Region, Error>")

  let member = try spelledType(
    fragments([("Plugin", "s:4Demo6PluginC"), (".", nil), ("Region", "s:4Demo6PluginC6RegionV")]))
  #expect(member == .name("Plugin.Region", identifier: "s:4Demo6PluginC6RegionV", arguments: []))
  #expect(try spelledType(fragments([("()", nil)])) == .tuple([]))
}

@Test func readsTypesARegistrationSpells() throws {
  let identifiers = ["String": "s:SS", "Plugin.Region": "s:4Demo6PluginC6RegionV"]
  let record = try spelledType("[String: Plugin.Region]?") { identifiers[$0] }
  #expect(
    record
      == .optional(
        .dictionary(
          key: .name("String", identifier: "s:SS", arguments: []),
          value: .name("Plugin.Region", identifier: "s:4Demo6PluginC6RegionV", arguments: []))))
  #expect(try spelledType("[Missing]") { identifiers[$0] } == nil)
}

@Test func rejectsTypesTheWorkerCannotPass() {
  for parts in [[("(Int) -> Void", nil as String?)], [("Int", "s:Si"), ("...", nil)]] {
    #expect(throws: DescribeError.self) { try spelledType(fragments(parts)) }
  }
}
