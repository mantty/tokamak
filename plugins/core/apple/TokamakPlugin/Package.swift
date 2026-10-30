// swift-tools-version: 5.9

import PackageDescription

// The deployment targets are the platform pack's, in
// platforms/apple/build/*-deployment-target.
let package = Package(
  name: "TokamakPlugin",
  platforms: [.iOS("17.0"), .macOS("14.0")],
  products: [.library(name: "TokamakPlugin", targets: ["TokamakPlugin"])],
  targets: [.target(name: "TokamakPlugin")]
)
