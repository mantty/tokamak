// swift-tools-version: 6.0
import PackageDescription

let package = Package(
  name: "TokamakDescriber",
  platforms: [.macOS(.v14)],
  products: [
    .executable(name: "tokamak-swift-describe", targets: ["tokamak-swift-describe"])
  ],
  dependencies: [
    .package(url: "https://github.com/swiftlang/swift-syntax.git", exact: "603.0.2")
  ],
  targets: [
    .target(
      name: "Describer",
      dependencies: [
        .product(name: "SwiftParser", package: "swift-syntax"),
        .product(name: "SwiftSyntax", package: "swift-syntax"),
      ]),
    .executableTarget(name: "tokamak-swift-describe", dependencies: ["Describer"]),
    .testTarget(name: "DescriberTests", dependencies: ["Describer"]),
  ]
)
