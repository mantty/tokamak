// swift-tools-version: 5.9

import PackageDescription

let package = Package(
  name: "TokamakSecureStorage",
  platforms: [.iOS("17.0"), .macOS("14.0")],
  products: [.library(name: "TokamakSecureStorage", targets: ["TokamakSecureStorage"])],
  dependencies: [.package(path: "../node_modules/@tokamakdev/plugin/apple/TokamakPlugin")],
  targets: [
    .target(
      name: "TokamakSecureStorage", dependencies: [.product(name: "TokamakPlugin", package: "TokamakPlugin")])
  ]
)
