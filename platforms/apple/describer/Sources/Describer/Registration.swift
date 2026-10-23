import SwiftParser
import SwiftSyntax

/// A failure to describe a plugin, which the message explains.
public struct DescribeError: Error, CustomStringConvertible {
  public let description: String

  init(_ description: String) {
    self.description = description
  }
}

/// How the Worker calls an exported method.
enum ExportKind: String {
  case call, stream, resource
}

/// A method the registration file lists.
struct Registration: Equatable {
  let kind: ExportKind
  let name: String
  /// The argument labels the method is named with, as in `addRegion(_:replace:)`.
  let labels: [String]?
  let doc: String
  let dropsOldest: Bool
}

/// An event the registration file lists.
struct EventRegistration: Equatable {
  let name: String
  /// The type listeners receive, as the registration spells it, such as `Update`.
  let type: String
  /// The type listeners reply with, as the registration spells it.
  let reply: String?
  let doc: String
}

/// The plugin type the registration file extends, and the methods and events it lists.
struct Registrations {
  let plugin: String
  let methods: [Registration]
  let events: [EventRegistration]
}

/// Reads the registration file `source`: an extension of the plugin whose
/// `static func exports(_ e: inout TokamakExporter<Plugin>)` calls `e.call`,
/// `e.stream` or `e.resource` once per method, and `e.event` once per event,
/// with literal options.
func readRegistrations(_ source: String, file: String) throws(DescribeError) -> Registrations {
  let tree = Parser.parse(source: source)
  let converter = SourceLocationConverter(fileName: file, tree: tree)
  let fail = { (node: any SyntaxProtocol, message: String) -> DescribeError in
    let location = node.startLocation(converter: converter)
    return DescribeError("\(file):\(location.line):\(location.column): \(message)")
  }
  let exports = tree.statements.compactMap { item -> (ExtensionDeclSyntax, FunctionDeclSyntax)? in
    guard let extensionDecl = item.item.as(ExtensionDeclSyntax.self) else { return nil }
    let function = extensionDecl.memberBlock.members.lazy
      .compactMap { $0.decl.as(FunctionDeclSyntax.self) }
      .first { $0.name.text == "exports" }
    return function.map { (extensionDecl, $0) }
  }
  guard exports.count == 1, let (extensionDecl, function) = exports.first else {
    throw DescribeError("\(file): declare exports(_:) in one extension of the plugin")
  }
  guard let parameter = function.signature.parameterClause.parameters.first,
    function.signature.parameterClause.parameters.count == 1, let body = function.body
  else {
    throw fail(function, "exports(_:) takes the exporter and has a body")
  }
  let exporter = (parameter.secondName ?? parameter.firstName).text
  var methods: [Registration] = []
  var events: [EventRegistration] = []
  for item in statements(body.statements) {
    guard let call = item.as(FunctionCallExprSyntax.self),
      let member = call.calledExpression.as(MemberAccessExprSyntax.self),
      member.base?.as(DeclReferenceExprSyntax.self)?.baseName.text == exporter,
      call.trailingClosure == nil
    else {
      throw fail(
        item,
        "list each method with \(exporter).call, \(exporter).stream or \(exporter).resource, and each event with \(exporter).event"
      )
    }
    let name = member.declName.baseName.text
    if name == "event" {
      events.append(try eventRegistration(call, exporter: exporter, fail: fail))
    } else {
      methods.append(try registration(call, named: name, exporter: exporter, fail: fail))
    }
  }
  return Registrations(
    plugin: extensionDecl.extendedType.trimmedDescription, methods: methods, events: events)
}

/// The statements of `items`, including those in every clause of an `#if`,
/// which registers a method only some platforms have.
private func statements(_ items: CodeBlockItemListSyntax) -> [CodeBlockItemSyntax.Item] {
  items.flatMap { item -> [CodeBlockItemSyntax.Item] in
    guard let conditional = item.item.as(IfConfigDeclSyntax.self) else { return [item.item] }
    return conditional.clauses.flatMap { clause -> [CodeBlockItemSyntax.Item] in
      guard case .statements(let items) = clause.elements else { return [] }
      return statements(items)
    }
  }
}

/// The method registration that `call`, `exporter.<name>(method, ...)`, makes.
private func registration(
  _ call: FunctionCallExprSyntax, named name: String, exporter: String,
  fail: (any SyntaxProtocol, String) -> DescribeError
) throws(DescribeError) -> Registration {
  guard let kind = ExportKind(rawValue: name) else {
    throw fail(call, "\(exporter).\(name) is not call, stream, resource or event")
  }
  guard let first = call.arguments.first, first.label == nil,
    let reference = first.expression.as(DeclReferenceExprSyntax.self)
  else {
    throw fail(call, "name the method first, as in \(exporter).\(kind)(addRegion)")
  }
  var doc = ""
  var dropsOldest = false
  for argument in call.arguments.dropFirst() {
    switch (argument.label?.text, kind) {
    case ("doc", _):
      doc = try documentation(argument, fail: fail)
    case ("overflow", .stream):
      let value = argument.expression.as(MemberAccessExprSyntax.self)
      guard let value, value.base == nil,
        ["error", "dropOldest"].contains(value.declName.baseName.text)
      else {
        throw fail(argument, "overflow is .error or .dropOldest")
      }
      dropsOldest = value.declName.baseName.text == "dropOldest"
    default:
      throw fail(
        argument, "\(kind) takes the method, then doc\(kind == .stream ? " and overflow" : "")")
    }
  }
  return Registration(
    kind: kind,
    name: reference.baseName.text,
    labels: reference.argumentNames?.arguments.map(\.name.text),
    doc: doc,
    dropsOldest: dropsOldest
  )
}

/// The event that `call`, `exporter.event("name", Type.self, ...)`, lists.
private func eventRegistration(
  _ call: FunctionCallExprSyntax, exporter: String,
  fail: (any SyntaxProtocol, String) -> DescribeError
) throws(DescribeError) -> EventRegistration {
  let arguments = Array(call.arguments)
  guard arguments.count >= 2, arguments[0].label == nil, arguments[1].label == nil,
    let name = arguments[0].expression.as(StringLiteralExprSyntax.self)?.representedLiteralValue,
    let type = metatype(arguments[1].expression)
  else {
    throw fail(
      call, "name the event, then its type, as in \(exporter).event(\"update\", Update.self)")
  }
  var reply: String?
  var doc = ""
  for argument in arguments.dropFirst(2) {
    switch argument.label?.text {
    case "reply":
      guard let type = metatype(argument.expression) else {
        throw fail(argument, "reply names a type, as in Answer.self")
      }
      reply = type
    case "doc":
      doc = try documentation(argument, fail: fail)
    default:
      throw fail(argument, "event takes the name and type, then reply and doc")
    }
  }
  return EventRegistration(name: name, type: type, reply: reply, doc: doc)
}

/// The type that `expression`, such as `Update.self`, names, as its source spells it.
private func metatype(_ expression: ExprSyntax) -> String? {
  guard let member = expression.as(MemberAccessExprSyntax.self),
    member.declName.baseName.tokenKind == .keyword(.self), let base = member.base
  else {
    return nil
  }
  return base.trimmedDescription
}

/// The value of a `doc` argument: a string literal without interpolation.
private func documentation(
  _ argument: LabeledExprSyntax, fail: (any SyntaxProtocol, String) -> DescribeError
) throws(DescribeError) -> String {
  guard let literal = argument.expression.as(StringLiteralExprSyntax.self),
    let value = literal.representedLiteralValue
  else {
    throw fail(argument, "doc is a string literal without interpolation")
  }
  return value
}
