import Foundation
import WebKit

extension URL {
  /// Whether this URL is on the app origin, `https://<host>`.
  func isAppOrigin(_ host: String) -> Bool {
    scheme?.caseInsensitiveCompare("https") == .orderedSame
      && self.host?.caseInsensitiveCompare(host) == .orderedSame
      && (port == nil || port == 443)
  }
}

extension Bundle {
  /// The host of the app origin, `https://<host>`, from the `TokamakHost` key.
  var tokamakHost: String? {
    object(forInfoDictionaryKey: "TokamakHost") as? String
  }
}

extension WKSecurityOrigin {
  /// Whether this is the app origin, `https://<host>`.
  func isAppOrigin(_ host: String) -> Bool {
    self.protocol.caseInsensitiveCompare("https") == .orderedSame
      && self.host.caseInsensitiveCompare(host) == .orderedSame
      && (port == 0 || port == 443)
  }
}
