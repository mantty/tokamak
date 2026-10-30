import Describer
import Foundation

// tokamak-swift-describe EXPORTS OUTPUT PLATFORM=SYMBOL-GRAPH...
//
// Describes the methods the registration file EXPORTS lists, resolved in each
// platform's symbol graphs, into OUTPUT/description.json, and writes the
// dispatch that serves them to OUTPUT/Exports.generated.swift. A platform
// named more than once has each of its graphs.
let arguments = CommandLine.arguments.dropFirst()
guard arguments.count >= 3 else {
  FileHandle.standardError.write(
    Data("usage: tokamak-swift-describe EXPORTS OUTPUT PLATFORM=SYMBOL-GRAPH...\n".utf8))
  exit(2)
}
let exports = URL(fileURLWithPath: arguments[arguments.startIndex])
let output = URL(fileURLWithPath: arguments[arguments.startIndex + 1])
do {
  var symbolGraphs: [(platform: String, graphs: [Data])] = []
  for argument in arguments.dropFirst(2) {
    let platform = String(argument.prefix { $0 != "=" })
    let graph = try Data(
      contentsOf: URL(fileURLWithPath: String(argument.drop { $0 != "=" }.dropFirst())))
    if let index = symbolGraphs.firstIndex(where: { $0.platform == platform }) {
      symbolGraphs[index].graphs.append(graph)
    } else {
      symbolGraphs.append((platform, [graph]))
    }
  }
  let described = try describe(
    exports: try String(contentsOf: exports, encoding: .utf8), file: exports.path,
    symbolGraphs: symbolGraphs)
  try described.description.write(to: output.appendingPathComponent("description.json"))
  try Data(described.dispatch.utf8).write(
    to: output.appendingPathComponent("Exports.generated.swift"))
} catch {
  FileHandle.standardError.write(Data("\(error)\n".utf8))
  exit(1)
}
