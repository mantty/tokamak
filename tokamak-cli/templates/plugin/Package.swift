// swift-tools-version: 5.9

import PackageDescription

let package = Package(
  name: "{{module}}",
  platforms: [.iOS("17.0"), .macOS("14.0")],
  products: [.library(name: "{{module}}", targets: ["{{module}}"])],
  dependencies: [.package(path: "../node_modules/@tokamakdev/plugin/apple/TokamakPlugin")],
  targets: [
    .target(
      name: "{{module}}", dependencies: [.product(name: "TokamakPlugin", package: "TokamakPlugin")])
  ]
)
