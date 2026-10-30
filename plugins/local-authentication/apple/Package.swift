// swift-tools-version: 5.9

import PackageDescription

let package = Package(
  name: "TokamakLocalAuthentication",
  platforms: [.iOS("17.0"), .macOS("14.0")],
  products: [.library(name: "TokamakLocalAuthentication", targets: ["TokamakLocalAuthentication"])],
  dependencies: [.package(path: "../node_modules/@tokamakdev/plugin/apple/TokamakPlugin")],
  targets: [
    .target(
      name: "TokamakLocalAuthentication",
      dependencies: [.product(name: "TokamakPlugin", package: "TokamakPlugin")])
  ]
)
