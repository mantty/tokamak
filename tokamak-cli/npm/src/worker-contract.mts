import path from "node:path";
import fs from "node:fs";
import { fileURLToPath } from "node:url";
import ts from "typescript";

import { checkRpcMethod } from "./rpc-contract.mjs";
import { serviceTypes, type ServiceMethod } from "./service-types.mjs";

/** The callable prototype methods and source dependencies of a Worker entrypoint. */
export interface WorkerContract {
  methods: string[];
  files: string[];
  declarations: string;
}

export function findModule(root: string, name: string | undefined): string | undefined {
  const candidates = (name ? [name] : ["src/tokamak.ts", "src/tokamak.js"])
    .map(candidate => path.resolve(root, candidate).split(path.sep).join("/"));
  const file = candidates.find(candidate => fs.existsSync(candidate));
  if (name && !file) throw new Error(`tokamak module not found: ${candidates[0]}`);
  return file;
}

/** Resolve the application's types before generating its service adapters. */
export function readWorkerContract(entry: string, root: string): WorkerContract {
  return readWorkerContracts(entry, undefined, root, true).native;
}

/** Check every WorkerEntrypoint exported by an application service bundle. */
export function readServiceContracts(entry: string, root: string): Record<string, WorkerContract> {
  return readWorkerContracts(undefined, entry, root, true).main;
}

/** Analyze both entrypoints in one TypeScript program. */
export function readWorkerContracts(entry: string | undefined, main: string | undefined, root: string, validate: boolean): {
  native: WorkerContract;
  main: Record<string, WorkerContract>;
} {
  const entries = [entry, main].filter((file): file is string => !!file);
  const project = entries.length ? workerProgram(entries, root) : undefined;
  return {
    native: entry && project ? classContract(workerModule(entry, project), "default", true, validate)
      : { methods: [], files: [], declarations: serviceTypes("{}", []) },
    main: main && project ? serviceContracts(workerModule(main, project), validate) : {},
  };
}

function serviceContracts(module: ReturnType<typeof workerModule>, validate: boolean): Record<string, WorkerContract> {
  const { entry } = module;
  const contracts: Record<string, WorkerContract> = {};
  for (const [name, type] of module.exports) {
    const instance = type.getConstructSignatures()[0]?.getReturnType();
    if (instance && applicationPrototypes(module.program, module.checker, instance, entry)) {
      contracts[name] = classContract(module, name, false, validate);
    } else if (name === "default" && (type.flags & ts.TypeFlags.Object) && !type.getCallSignatures().length && !instance) {
      contracts[name] = { methods: [], files: module.files, declarations: serviceTypes("{}", [], "Main") };
    } else if (name === "default") {
      throw new Error(`Service entrypoint ${entry} must export a handler object or extend WorkerEntrypoint.`);
    }
  }
  return contracts;
}

function workerProgram(entries: string[], root: string) {
  const config = projectConfig(root);
  const program = ts.createProgram({
    rootNames: [...new Set([...entries, ...config.fileNames.filter((file) => /\.d\.[cm]?ts$/.test(file))])],
    options: config.options,
    projectReferences: config.projectReferences,
  });
  const syntax = program.getSyntacticDiagnostics();
  if (syntax.length) throw diagnostics(syntax);
  const checker = program.getTypeChecker();
  const files = [...new Set([
    ...config.configFiles,
    ...program.getSourceFiles().filter((file) => !program.isSourceFileDefaultLibrary(file)).map((file) => file.fileName),
  ])].sort();
  return { program, checker, files };
}

function workerModule(entry: string, project: ReturnType<typeof workerProgram>) {
  const { program, checker } = project;
  const source = program.getSourceFile(entry);
  if (!source) throw new Error(`Cannot read Tokamak entrypoint ${entry}.`);
  const module = checker.getSymbolAtLocation(source);
  const exports = new Map((module ? checker.getExportsOfModule(module) : []).map(exported => {
    const symbol = exported.flags & ts.SymbolFlags.Alias ? checker.getAliasedSymbol(exported) : exported;
    return [exported.name, checker.getTypeOfSymbolAtLocation(symbol, source)];
  }));
  return { ...project, entry, source, exports };
}

function classContract(module: ReturnType<typeof workerModule>, name: string, native: boolean, validate: boolean): WorkerContract {
  const { entry, program, checker, source } = module;
  const constructor = module.exports.get(name)?.getConstructSignatures()[0];
  const instance = constructor?.getReturnType();
  const declarations = instance?.symbol?.declarations ?? [];
  const concrete = declarations.some((node) =>
    (ts.isClassDeclaration(node) || ts.isClassExpression(node)) &&
    !ts.getModifiers(node)?.some((modifier) => modifier.kind === ts.SyntaxKind.AbstractKeyword));
  if (!instance || !concrete || constructor?.typeParameters?.length) {
    throw new Error(`Tokamak entrypoint ${entry} must default-export a concrete class.`);
  }
  const prototypes = applicationPrototypes(program, checker, instance, entry, native);
  if (!prototypes) {
    throw new Error(`Tokamak entrypoint ${entry} must extend TokamakEntrypoint from @tokamakdev/tok or WorkerEntrypoint from cloudflare:workers; include its type declarations.`);
  }
  const methods: ServiceMethod[] = [];
  for (const property of instance.getProperties()) {
    if (!property.declarations?.some((node) => prototypes.has(node.parent))) continue;
    if (!exposedMethod(property)) continue;
    const type = checker.getTypeOfSymbolAtLocation(property, source);
    if (validate) checkRpcMethod(checker, type, property.name);
    methods.push({
      name: property.name,
      overloads: type.getCallSignatures().length,
      documentation: [
        ts.displayPartsToString(property.getDocumentationComment(checker)),
        ...property.getJsDocTags(checker).map((tag) => `@${tag.name} ${ts.displayPartsToString(tag.text)}`.trim()),
      ].filter(Boolean).join("\n"),
    });
  }
  methods.sort((left, right) => left.name < right.name ? -1 : left.name > right.name ? 1 : 0);
  return {
    methods: methods.map((method) => method.name),
    declarations: serviceTypes(
      `InstanceType<typeof import(${JSON.stringify(entry.split(path.sep).join("/"))}, { with: { "resolution-mode": "import" } })[${JSON.stringify(name)}]>`,
      methods,
      native ? "Tokamak" : "Main",
    ),
    files: module.files,
  };
}

function applicationPrototypes(program: ts.Program, checker: ts.TypeChecker, instance: ts.Type, entry: string, allowNative = false): Set<ts.Node> | undefined {
  const module = checker.getAmbientModules().find((symbol) => symbol.name === '"cloudflare:workers"');
  const exported = module && checker.getExportsOfModule(module).find((symbol) => symbol.name === "WorkerEntrypoint");
  const base = exported && (exported.flags & ts.SymbolFlags.Alias ? checker.getAliasedSymbol(exported) : exported);
  const native = allowNative ? nativeEntrypoint(program, checker, entry) : undefined;
  const prototypes = new Set<ts.Node>();
  let current: ts.Type | undefined = instance;
  const seen = new Set<ts.Type>();
  while (current && !seen.has(current)) {
    const symbol = current.getSymbol();
    if (symbol && (symbol === base || symbol === native)) return prototypes;
    seen.add(current);
    for (const node of current.getSymbol()?.declarations ?? []) prototypes.add(node);
    current = checker.getBaseTypes(current as ts.InterfaceType)?.[0];
  }
  return undefined;
}

function nativeEntrypoint(program: ts.Program, checker: ts.TypeChecker, entry: string): ts.Symbol | undefined {
  const options = program.getCompilerOptions();
  const sdk = ts.resolveModuleName("@tokamakdev/tok", entry, options, ts.sys).resolvedModule?.resolvedFileName;
  const core = ts.resolveModuleName("@tokamakdev/plugin/worker", sdk ?? fileURLToPath(import.meta.url), options, ts.sys).resolvedModule;
  const source = core && program.getSourceFile(core.resolvedFileName);
  const module = source && checker.getSymbolAtLocation(source);
  return module && checker.getExportsOfModule(module).find((symbol) => symbol.name === "TokamakEntrypoint");
}

function exposedMethod(property: ts.Symbol): boolean {
  const declarations = property.declarations?.filter(ts.isMethodDeclaration) ?? [];
  if (!declarations.length || declarations.some((node) => ts.isPrivateIdentifier(node.name)) ||
    String(property.escapedName).startsWith("__@")) return false;
  if (declarations.some((node) => ts.getModifiers(node)?.some((modifier) =>
    modifier.kind === ts.SyntaxKind.PrivateKeyword || modifier.kind === ts.SyntaxKind.ProtectedKeyword))) {
    throw new Error(`Method ${property.name} is on the public JavaScript prototype; use a #private method for a local helper.`);
  }
  if (["then", "connect", "dup", "constructor", "alarm", "webSocketMessage", "webSocketClose", "webSocketError", "__tokamakRpc"].includes(property.name)) {
    throw new Error(`Method ${property.name} is reserved by the service binding contract.`);
  }
  return !["fetch", "event", "queue", "scheduled", "email", "tail", "trace", "tailStream", "test"].includes(property.name);
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
