import TokamakPlugin

extension TokamakExamplePlugin: TokamakExports {
  public static func exports(_ e: inout TokamakExporter<TokamakExamplePlugin>) {
    e.call(addRegion, doc: "Monitors a circular region until `removeRegion`.")
    e.call(regions)
    #if os(iOS)
      e.call(permission, doc: "The app's permission.\n\nNever asks.")
    #endif
    e.stream(watchPosition, overflow: .dropOldest)
    e.stream(onVisit)
    e.resource(capture)
    e.call(encode)
    e.call(upload)
    e.call(digest)
    e.call(read)
    e.call(copy)
    e.call(mirror)
    e.call(alert)
    e.call(zone)
    e.call(mark)
    e.event("onEntered", Region.self, doc: "Each region the device enters.")
    e.event("onAsked", Region.self, reply: Precision.self)
    e.event("onChose", Precision.self)
    #if os(iOS)
      e.event("onVisited", Visit.self)
    #endif
  }
}
