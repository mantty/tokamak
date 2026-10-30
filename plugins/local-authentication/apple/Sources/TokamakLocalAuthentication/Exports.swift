import TokamakPlugin

extension TokamakLocalAuthenticationPlugin: TokamakExports {
  public static func exports(_ e: inout TokamakExporter<TokamakLocalAuthenticationPlugin>) {
    e.call(status, doc: "Whether the device can perform `authentication` now.")
    e.call(
      authenticate, doc: "Asks the device owner to authenticate, giving `prompt` as the reason.")
  }
}
