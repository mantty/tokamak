// swift-tools-version: 5.9

import PackageDescription

let package = Package(
  name: "TokamakExample",
  platforms: [.iOS("17.0"), .macOS("14.0")],
  products: [.library(name: "TokamakExample", targets: ["TokamakExample"])],
  dependencies: [.package(path: "../node_modules/@tokamakdev/plugin/apple/TokamakPlugin")],
  targets: [
    .target(
      name: "TokamakExample", dependencies: [.product(name: "TokamakPlugin", package: "TokamakPlugin")])
  ]
)
