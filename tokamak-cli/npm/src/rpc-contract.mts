import ts from "typescript";

const scalars = ts.TypeFlags.StringLike | ts.TypeFlags.NumberLike | ts.TypeFlags.BooleanLike |
  ts.TypeFlags.BigIntLike | ts.TypeFlags.Null | ts.TypeFlags.Undefined | ts.TypeFlags.Never;
const binaryViews = new Set([
  "Int8Array", "Uint8Array", "Uint8ClampedArray", "Int16Array", "Uint16Array",
  "Int32Array", "Uint32Array", "Float16Array", "Float32Array", "Float64Array",
  "BigInt64Array", "BigUint64Array", "DataView",
]);
const errors = new Set(["Error", "DOMException", "AggregateError", "EvalError", "RangeError", "ReferenceError", "SyntaxError", "TypeError", "URIError"]);

interface Traversal {
  seen: Set<ts.Type>;
  depth: number;
  workerOnly?: ts.Symbol;
}

/** A method that passes a value only the Tokamak worker can hold. */
export class WorkerOnlyMethod extends Error {}

/**
 * Validates every overload before generating a remote method. A signature containing the
 * `workerOnly` type throws `WorkerOnlyMethod`.
 */
export function checkRpcMethod(checker: ts.TypeChecker, method: ts.Type, name: string, workerOnly?: ts.Symbol): void {
  const signatures = method.getCallSignatures();
  if (!signatures.length) reject(checker, method, name, "expected a callable method");
  for (const signature of signatures) checkSignature(checker, signature, name, workerOnly);
}

function checkSignature(checker: ts.TypeChecker, signature: ts.Signature, name: string, workerOnly?: ts.Symbol): void {
  if (signature.typeParameters?.length) {
    throw new Error(`Unsupported RPC signature at ${name}: generic methods need a concrete service signature.`);
  }
  const traversal: Traversal = { seen: new Set(), depth: 0, workerOnly };
  for (const parameter of signature.parameters) {
    checkValue(checker, checker.getTypeOfSymbol(parameter), `${name}.${parameter.name}`, traversal);
  }
  const result = checker.getAwaitedType(signature.getReturnType());
  if (!result) throw new Error(`Unsupported RPC result at ${name}.result: cannot resolve the awaited result.`);
  for (const member of result.isUnion() ? result.types : [result]) {
    if (!(member.flags & ts.TypeFlags.Void)) checkValue(checker, member, `${name}.result`, traversal);
  }
}

function reject(checker: ts.TypeChecker, type: ts.Type, path: string, reason?: string): never {
  throw new Error(`Unsupported RPC type at ${path}: ${reason ?? checker.typeToString(type)}.`);
}

/** Global ambient types identify host values; module-local names do not. */
function builtin(checker: ts.TypeChecker, type: ts.Type): string | undefined {
  const symbol = type.getSymbol();
  if (!symbol) return;
  if (symbol.name === "ReadableStream" && nodeReadableStream(checker, symbol)) return symbol.name;
  if (symbol !== checker.resolveName(symbol.name, undefined, ts.SymbolFlags.Type, false)) return;
  const ambient = symbol.declarations?.some((declaration) => declaration.getSourceFile().isDeclarationFile);
  const value = checker.resolveName(symbol.name, undefined, ts.SymbolFlags.Value, false);
  return ambient && (value || ["ReadonlyArray", "ReadonlyMap", "ReadonlySet"].includes(symbol.name)) ? symbol.name : undefined;
}

function nodeReadableStream(checker: ts.TypeChecker, symbol: ts.Symbol): boolean {
  return checker.getAmbientModules().filter((module) => ["\"stream/web\"", "\"node:stream/web\""].includes(module.name))
    .some((module) => checker.getExportsOfModule(module).some((member) =>
      member.name === "ReadableStream" &&
      aliased(checker, member) === symbol));
}

/** The symbol that `symbol` names: itself, or the declaration an import or re-export refers to. */
export function aliased(checker: ts.TypeChecker, symbol: ts.Symbol): ts.Symbol {
  return symbol.flags & ts.SymbolFlags.Alias ? checker.getAliasedSymbol(symbol) : symbol;
}

function argumentsOf(checker: ts.TypeChecker, type: ts.Type): readonly ts.Type[] {
  return type.flags & ts.TypeFlags.Object && (type as ts.ObjectType).objectFlags & ts.ObjectFlags.Reference
    ? checker.getTypeArguments(type as ts.TypeReference) : [];
}

function checkValue(checker: ts.TypeChecker, type: ts.Type, path: string, traversal: Traversal): void {
  if (type.flags & scalars || traversal.seen.has(type)) return;
  if (traversal.depth >= 100) reject(checker, type, path, "type expansion is too deep to verify; use a finite recursive data shape");
  traversal.seen.add(type);
  traversal.depth++;
  try {
    checkObject(checker, type, path, traversal);
  } finally {
    traversal.depth--;
  }
}

function checkObject(checker: ts.TypeChecker, type: ts.Type, path: string, traversal: Traversal): void {
  if (type.isIntersection() && type.types.some((member) => Boolean(member.flags & scalars)) &&
    !type.getCallSignatures().length && !type.getConstructSignatures().length) return;
  if (type.isUnionOrIntersection()) {
    for (const member of type.types) checkValue(checker, member, path, traversal);
    return;
  }
  if (!(type.flags & ts.TypeFlags.Object)) reject(checker, type, path);
  if (traversal.workerOnly && type.getSymbol() === traversal.workerOnly) {
    throw new WorkerOnlyMethod(`${path} is ${checker.typeToString(type)}, which stays in the Tokamak worker.`);
  }
  if (type.getCallSignatures().length || type.getConstructSignatures().length) reject(checker, type, path);
  const name = builtin(checker, type);
  if (name === "Date" || name === "RegExp" || name === "ArrayBuffer") return;
  if (name && binaryViews.has(name)) return checkBuffer(checker, type, path);
  if (name === "ReadableStream") return checkStream(checker, type, path);
  if (checker.isArrayType(type) || checker.isTupleType(type) || name === "ReadonlyArray") {
    checkArguments(checker, type, path, traversal);
    return;
  }
  if (["Map", "ReadonlyMap", "Set", "ReadonlySet"].includes(name ?? "")) {
    checkArguments(checker, type, path, traversal);
    return;
  }
  if (isError(checker, type)) return checkProperties(checker, type, path, traversal, true);
  const classType = type.getSymbol()?.declarations?.some((node) => ts.isClassDeclaration(node) || ts.isClassExpression(node));
  if (name || classType) reject(checker, type, path);
  checkProperties(checker, type, path, traversal);
}

function isError(checker: ts.TypeChecker, type: ts.Type): boolean {
  if (errors.has(builtin(checker, type) ?? "")) return true;
  if (!(type.flags & ts.TypeFlags.Object)) return false;
  const object = type as ts.ObjectType;
  const target = object.objectFlags & ts.ObjectFlags.Reference ? (type as ts.TypeReference).target : object;
  return Boolean(target.objectFlags & ts.ObjectFlags.ClassOrInterface) &&
    checker.getBaseTypes(target as ts.InterfaceType).some((base) => isError(checker, base));
}

function checkArguments(checker: ts.TypeChecker, type: ts.Type, path: string, traversal: Traversal): void {
  for (const [index, argument] of argumentsOf(checker, type).entries()) {
    checkValue(checker, argument, `${path}[${index}]`, traversal);
  }
}

function checkBuffer(checker: ts.TypeChecker, type: ts.Type, path: string): void {
  const [buffer] = argumentsOf(checker, type);
  // Default ArrayBufferLike views require the runtime's non-shared-buffer check.
  if (buffer && builtin(checker, buffer) === "SharedArrayBuffer") reject(checker, type, path, "shared buffers cannot cross RPC");
  if (buffer && buffer.flags & (ts.TypeFlags.Any | ts.TypeFlags.Unknown | ts.TypeFlags.TypeParameter)) reject(checker, buffer, path);
}

function checkStream(checker: ts.TypeChecker, type: ts.Type, path: string): void {
  const [chunk] = argumentsOf(checker, type);
  if (!chunk || builtin(checker, chunk) !== "Uint8Array") {
    reject(checker, type, path, "readable streams must contain Uint8Array chunks");
  }
  checkBuffer(checker, chunk, path);
}

function checkProperties(
  checker: ts.TypeChecker, type: ts.Type, path: string, traversal: Traversal, error = false,
): void {
  const properties = type.getProperties();
  const indexes = checker.getIndexInfosOfType(type);
  if (!error && !properties.length && !indexes.length) reject(checker, type, path, "an unconstrained object is not a data shape");
  for (const property of properties) {
    if (String(property.escapedName).startsWith("__@")) reject(checker, type, path, "symbol properties cannot cross RPC");
    if (error && standardErrorField(checker, property)) continue;
    checkValue(checker, checker.getTypeOfSymbol(property), `${path}.${property.name}`, traversal);
  }
  for (const index of indexes) {
    if (index.keyType.flags & ts.TypeFlags.ESSymbolLike) reject(checker, type, path, "symbol indexes cannot cross RPC");
    checkValue(checker, index.type, `${path}[key]`, traversal);
  }
}

function standardErrorField(checker: ts.TypeChecker, property: ts.Symbol): boolean {
  return property.declarations?.some(({ parent }) =>
    (ts.isInterfaceDeclaration(parent) || ts.isClassDeclaration(parent)) &&
    errors.has(builtin(checker, checker.getTypeAtLocation(parent)) ?? "")) ?? false;
}
