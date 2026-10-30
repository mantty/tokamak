import TokamakPlugin

extension {{class}}: TokamakExports {
  public static func exports(_ e: inout TokamakExporter<{{class}}>) {
    e.call(echo, doc: "Returns a message through the native implementation.")
    e.event("echoed", Echo.self, doc: "Receives each message the native implementation echoed.")
  }
}
