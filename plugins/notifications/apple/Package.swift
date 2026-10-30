// swift-tools-version: 5.9

import PackageDescription

let package = Package(
  name: "TokamakNotifications",
  platforms: [.iOS("17.0"), .macOS("14.0")],
  products: [.library(name: "TokamakNotifications", targets: ["TokamakNotifications"])],
  dependencies: [.package(path: "../node_modules/@tokamakdev/plugin/apple/TokamakPlugin")],
  targets: [
    .target(
      name: "TokamakNotifications",
      dependencies: [.product(name: "TokamakPlugin", package: "TokamakPlugin")])
  ]
)
