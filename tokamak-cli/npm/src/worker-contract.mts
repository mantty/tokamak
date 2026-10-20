import path from "node:path";
import ts from "typescript";

import { checkRpcMethod } from "./rpc-contract.mjs";

/** The callable prototype methods and source dependencies of a Worker entrypoint. */
export interface WorkerContract {
  methods: string[];
  files: string[];
}

/** Resolve the application's types before generating its service adapters. */
export function readWorkerContract(entry: string, root: string): WorkerContract {
  const config = projectConfig(root);
  const program = ts.createProgram({
    rootNames: [...new Set([entry, ...config.fileNames.filter((file) => /\.d\.[cm]?ts$/.test(file))])],
    options: config.options,
    projectReferences: config.projectReferences,
  });
  const source = program.getSourceFile(entry);
  if (!source) throw new Error(`Cannot read Tokamak entrypoint ${entry}.`);
  const syntax = program.getSyntacticDiagnostics();
  if (syntax.length) throw diagnostics(syntax);
  const checker = program.getTypeChecker();
  const module = checker.getSymbolAtLocation(source);
  const exported = module && checker.getExportsOfModule(module).find((symbol) => symbol.name === "default");
  const symbol = exported && (exported.flags & ts.SymbolFlags.Alias ? checker.getAliasedSymbol(exported) : exported);
  const constructor = symbol && checker.getTypeOfSymbolAtLocation(symbol, source).getConstructSignatures()[0];
  const instance = constructor?.getReturnType();
  const declarations = instance?.symbol?.declarations ?? [];
  const concrete = declarations.some((node) =>
    (ts.isClassDeclaration(node) || ts.isClassExpression(node)) &&
    !ts.getModifiers(node)?.some((modifier) => modifier.kind === ts.SyntaxKind.AbstractKeyword));
  if (!instance || !concrete || constructor?.typeParameters?.length) {
    throw new Error(`Tokamak entrypoint ${entry} must default-export a concrete class.`);
  }
  const prototypes = applicationPrototypes(checker, instance, entry);
  const methods: string[] = [];
  for (const property of instance.getProperties()) {
    if (!property.declarations?.some((node) => prototypes.has(node.parent))) continue;
    if (!exposedMethod(property)) continue;
    checkRpcMethod(checker, checker.getTypeOfSymbolAtLocation(property, source), property.name);
    methods.push(property.name);
  }
  return {
    methods: methods.sort(),
    files: [...new Set([
      ...config.configFiles,
      ...program.getSourceFiles().filter((file) => !program.isSourceFileDefaultLibrary(file)).map((file) => file.fileName),
    ])].sort(),
  };
}

function applicationPrototypes(checker: ts.TypeChecker, instance: ts.Type, entry: string): Set<ts.Node> {
  const module = checker.getAmbientModules().find((symbol) => symbol.name === '"cloudflare:workers"');
  const exported = module && checker.getExportsOfModule(module).find((symbol) => symbol.name === "WorkerEntrypoint");
  const base = exported && (exported.flags & ts.SymbolFlags.Alias ? checker.getAliasedSymbol(exported) : exported);
  const prototypes = new Set<ts.Node>();
  let current: ts.Type | undefined = instance;
  const seen = new Set<ts.Type>();
  while (current && !seen.has(current)) {
    if (base && current.getSymbol() === base) return prototypes;
    seen.add(current);
    for (const node of current.getSymbol()?.declarations ?? []) prototypes.add(node);
    current = checker.getBaseTypes(current as ts.InterfaceType)?.[0];
  }
  throw new Error(`Tokamak entrypoint ${entry} must extend WorkerEntrypoint from cloudflare:workers; include the Workers type declarations.`);
}

function exposedMethod(property: ts.Symbol): boolean {
  const declarations = property.declarations?.filter(ts.isMethodDeclaration) ?? [];
  if (!declarations.length || declarations.some((node) => ts.isPrivateIdentifier(node.name)) ||
    String(property.escapedName).startsWith("__@")) return false;
  if (declarations.some((node) => ts.getModifiers(node)?.some((modifier) =>
    modifier.kind === ts.SyntaxKind.PrivateKeyword || modifier.kind === ts.SyntaxKind.ProtectedKeyword))) {
    throw new Error(`Method ${property.name} is on the public JavaScript prototype; use a #private method for a local helper.`);
  }
  if (["then", "connect", "dup", "constructor", "alarm", "webSocketMessage", "webSocketClose", "webSocketError"].includes(property.name)) {
    throw new Error(`Method ${property.name} is reserved by the service binding contract.`);
  }
  return !["fetch", "event"].includes(property.name);
}

function projectConfig(root: string): ts.ParsedCommandLine & { configFiles: string[] } {
  const configFiles: string[] = [];
  const host = {
    ...ts.sys,
    readFile(file: string) { configFiles.push(file); return ts.sys.readFile(file); },
  };
  const file = ts.findConfigFile(root, ts.sys.fileExists);
  const loaded = file ? ts.readConfigFile(file, host.readFile) : { config: {} };
  if (loaded.error) throw diagnostics([loaded.error]);
  const config = ts.parseJsonConfigFileContent(loaded.config, host, file ? path.dirname(file) : root, undefined, file);
  // An explicit entrypoint can live outside a project's include patterns.
  const errors = config.errors.filter((error) => error.code !== 18003);
  if (errors.length) throw diagnostics(errors);
  config.options = {
    target: ts.ScriptTarget.ES2023,
    module: ts.ModuleKind.ESNext,
    moduleResolution: ts.ModuleResolutionKind.Bundler,
    strict: true,
    types: [],
    ...config.options,
    allowJs: true,
    checkJs: true,
    noEmit: true,
  };
  return { ...config, configFiles };
}

function diagnostics(errors: readonly ts.Diagnostic[]): Error {
  return new Error(ts.formatDiagnostics(errors, {
    getCanonicalFileName: (name) => name,
    getCurrentDirectory: ts.sys.getCurrentDirectory,
    getNewLine: () => "\n",
  }));
}
