// swift-tools-version: 5.9

import PackageDescription

let package = Package(
  name: "TokamakLocation",
  platforms: [.iOS("17.0"), .macOS("14.0")],
  products: [.library(name: "TokamakLocation", targets: ["TokamakLocation"])],
  dependencies: [.package(path: "../node_modules/@tokamakdev/plugin/apple/TokamakPlugin")],
  targets: [
    .target(
      name: "TokamakLocation", dependencies: [.product(name: "TokamakPlugin", package: "TokamakPlugin")])
  ]
)
