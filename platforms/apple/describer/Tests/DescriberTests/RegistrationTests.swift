import Testing

@testable import Describer

private func registrations(_ body: String) throws(DescribeError) -> Registrations {
  try readRegistrations(
    """
    extension Plugin: TokamakExports {
      static func exports(_ e: inout TokamakExporter<Plugin>) {
        \(body)
      }
    }
    """, file: "Exports.swift")
}

@Test func readsEachRegistrationWithItsOptions() throws {
  let read = try registrations(
    """
    e.call(addRegion(_:replace:), doc: "Adds \\"it\\".\\nThen returns.")
    #if os(iOS)
      e.stream(watch, overflow: .dropOldest)
    #else
      e.resource(capture)
    #endif
    """)
  #expect(read.plugin == "Plugin")
  #expect(
    read.methods == [
      Registration(
        kind: .call, name: "addRegion", labels: ["_", "replace"],
        doc: "Adds \"it\".\nThen returns.",
        dropsOldest: false),
      Registration(kind: .stream, name: "watch", labels: nil, doc: "", dropsOldest: true),
      Registration(kind: .resource, name: "capture", labels: nil, doc: "", dropsOldest: false),
    ])
}

@Test func readsEachEventWithItsTypes() throws {
  let read = try registrations(
    """
    e.call(regions)
    e.event("entered", Plugin.Region.self, doc: "Each region entered.")
    e.event("asked", Region.self, reply: [String: TokamakJSON].self)
    """)
  #expect(read.methods.map(\.name) == ["regions"])
  #expect(
    read.events == [
      EventRegistration(
        name: "entered", type: "Plugin.Region", reply: nil, doc: "Each region entered."),
      EventRegistration(name: "asked", type: "Region", reply: "[String: TokamakJSON]", doc: ""),
    ])
}

@Test func rejectsWhatItCannotReadWithoutRunningTheFile() {
  let cases = [
    ("e.event(name, Region.self)", "name the event, then its type"),
    ("e.event(\"entered\", Region)", "name the event, then its type"),
    ("e.event(\"asked\", Region.self, reply: answer)", "reply names a type"),
    ("e.event(\"entered\", Region.self, overflow: .dropOldest)", "event takes the name and type"),
    ("e.call(addRegion, doc: doc)", "doc is a string literal"),
    ("e.call(addRegion, doc: \"\\(name)\")", "doc is a string literal"),
    ("e.call(addRegion, overflow: .dropOldest)", "call takes the method, then doc"),
    ("e.stream(watch, overflow: Overflow.dropOldest)", "overflow is .error or .dropOldest"),
    ("other.call(addRegion)", "list each method with e.call"),
    ("e.call({ $0.addRegion })", "name the method first"),
    ("print(1)", "list each method with e.call"),
  ]
  for (body, message) in cases {
    #expect {
      try registrations(body)
    } throws: { error in
      "\(error)".contains(message) && "\(error)".contains("Exports.swift:")
    }
  }
}

@Test func requiresOneExportsFunction() {
  #expect {
    try readRegistrations("extension Plugin {}", file: "Exports.swift")
  } throws: { error in
    "\(error)".contains("declare exports(_:) in one extension of the plugin")
  }
}
