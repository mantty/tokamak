import path from "node:path";
import fs from "node:fs";
import { fileURLToPath } from "node:url";
import ts from "typescript";

import { aliased, checkRpcMethod, WorkerOnlyMethod } from "./rpc-contract.mjs";
import { serviceTypes, type ServiceMethod } from "./service-types.mjs";
import type { NativePlatform, PluginDefinition } from "./plugin-manifest.mjs";

/** The callable prototype methods and source dependencies of a Worker entrypoint. */
export interface WorkerContract {
  methods: string[];
  files: string[];
  declarations: string;
}

export interface PluginContract extends WorkerContract {
  definition: PluginDefinition;
  webMethods: string[];
  /**
   * The native platforms that implement each operation of the native SDK, including those only the
   * Tokamak worker can call.
   */
  operations: Record<string, NativePlatform[]>;
}

/** Worker handler names, which are not RPC methods. */
const HANDLERS = ["fetch", "event", "queue", "scheduled", "email", "tail", "trace", "tailStream", "test"];

/** The contract of a Tokamak worker without a module. */
export function emptyContract(): WorkerContract {
  return { methods: [], files: [], declarations: serviceTypes("{}", []) };
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
export function readWorkerContracts(entry: string | undefined, main: string | undefined, root: string, validate: boolean, plugins: PluginDefinition[] = []): {
  native: WorkerContract;
  named: Record<string, WorkerContract>;
  main: Record<string, WorkerContract>;
  plugins: PluginContract[];
} {
  const entries = [entry, main, ...plugins.flatMap(plugin => [plugin.api.module, plugin.web?.module])]
    .filter((file): file is string => !!file);
  const project = entries.length ? workerProgram(entries, root) : undefined;
  const native = entry && project ? workerModule(entry, project) : undefined;
  return {
    native: native ? classContract(native, "default", true, validate) : emptyContract(),
    named: native ? namedContracts(native, validate) : {},
    main: main && project ? serviceContracts(workerModule(main, project), validate) : {},
    plugins: project ? plugins.map(plugin => pluginContract(workerModule(plugin.api.module, project), plugin, validate)) : [],
  };
}

/** The Tokamak worker's named entrypoints: its other exported concrete entrypoint classes. */
function namedContracts(module: ReturnType<typeof workerModule>, validate: boolean): Record<string, WorkerContract> {
  const contracts: Record<string, WorkerContract> = {};
  for (const [name, type] of module.exports) {
    const instance = type.getConstructSignatures()[0]?.getReturnType();
    const abstract = instance?.symbol?.declarations?.some(node =>
      ts.getModifiers(node as ts.ClassDeclaration)?.some(modifier => modifier.kind === ts.SyntaxKind.AbstractKeyword));
    if (name === "default" || !instance || abstract ||
      !applicationPrototypes(module.program, module.checker, instance, module.entry, true)) continue;
    if (name === "TokamakEvents" || name.startsWith("TOKAMAK_")) {
      throw new Error(`Tokamak entrypoint ${name} has a name the generated worker reserves.`);
    }
    contracts[name] = classContract(module, name, true, validate);
  }
  return contracts;
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
  const modules = new Map(entries.map(entry => [entry,
    ts.resolveModuleName(entry, path.join(root, "tokamak.ts"), config.options, ts.sys).resolvedModule?.resolvedFileName ?? entry,
  ]));
  const program = ts.createProgram({
    rootNames: [...new Set([...modules.values(), ...config.fileNames.filter((file) => /\.d\.[cm]?ts$/.test(file))])],
    options: config.options,
    projectReferences: config.projectReferences,
  });
  const syntax = program.getSyntacticDiagnostics();
  if (syntax.length) throw diagnostics(syntax);
  const checker = program.getTypeChecker();
  const files = [...new Set([
    ...entries,
    ...config.configFiles,
    ...program.getSourceFiles().filter((file) => !program.isSourceFileDefaultLibrary(file)).map((file) => file.fileName),
  ])].sort();
  return { program, checker, files, modules };
}

function workerModule(entry: string, project: ReturnType<typeof workerProgram>) {
  const { program, checker } = project;
  const source = program.getSourceFile(project.modules.get(entry) ?? entry);
  if (!source) throw new Error(`Cannot read Tokamak entrypoint ${entry}.`);
  const module = checker.getSymbolAtLocation(source);
  const exports = new Map((module ? checker.getExportsOfModule(module) : []).map(exported => {
    return [exported.name, checker.getTypeOfSymbolAtLocation(aliased(checker, exported), source)];
  }));
  return { ...project, entry, source, exports };
}

function pluginContract(module: ReturnType<typeof workerModule>, definition: PluginDefinition, validate: boolean): PluginContract {
  const { checker, program, source } = module;
  const signatures = module.exports.get(definition.api.export)?.getCallSignatures() ?? [];
  const signature = signatures[0];
  const parameter = signature?.parameters[0];
  const parameterType = parameter && checker.getTypeOfSymbol(parameter);
  const symbol = nativeSdkExport(program, checker, module.entry, "TokamakContext");
  const context = symbol && checker.getDeclaredTypeOfSymbol(symbol);
  if (signatures.length !== 1 || signature.typeParameters?.length || signature.parameters.length !== 1 ||
    !parameterType || parameterType.flags & (ts.TypeFlags.Any | ts.TypeFlags.Unknown) || !context ||
    !checker.isTypeAssignableTo(parameterType, context) || !checker.isTypeAssignableTo(context, parameterType)) {
    throw new Error(`Plugin ${definition.id} factory must accept one TokamakContext.`);
  }
  const api = signature.getReturnType();
  if (!apiObject(checker, api)) {
    throw new Error(`Plugin ${definition.id} factory must return a synchronous object with declared methods.`);
  }
  const reference = nativeSdkExport(program, checker, module.entry, "NativeReference", "@tokamakdev/plugin");
  const methods: ServiceMethod[] = [];
  const operations: Record<string, NativePlatform[]> = {};
  for (const property of api.getProperties()) {
    if (privateProperty(property)) continue;
    if (property.name === "capabilities") continue;
    assertMethodName(property.name);
    if (HANDLERS.includes(property.name)) {
      throw new Error(`Plugin method ${property.name} is reserved for a Worker handler.`);
    }
    const method = checker.getTypeOfSymbolAtLocation(property, source);
    if (!method.getCallSignatures().length || String(property.escapedName).startsWith("__@")) {
      throw new Error(`Plugin ${definition.id}.${property.name} must be a method.`);
    }
    operations[property.name] = implementedOn(property, definition);
    try {
      checkRpcMethod(checker, method, `${definition.id}.${property.name}`, reference);
    } catch (error) {
      // Methods that pass native references are available only in the Tokamak worker.
      if (error instanceof WorkerOnlyMethod) continue;
      if (validate) throw error;
    }
    // Its `@platforms` tag becomes the availability line the documentation ends with.
    methods.push(methodDeclaration(checker, property, method, ["platforms"]));
  }
  methods.sort(byName);
  const webMethods = browserMethods(module, definition, methods, validate);
  for (const method of methods) {
    method.documentation = [method.documentation, availability(operations[method.name], webMethods.includes(method.name))]
      .filter(Boolean).join("\n\n");
  }
  return {
    definition, methods: methods.map(method => method.name), files: [...module.files, definition.manifest],
    webMethods, operations: Object.fromEntries(Object.entries(operations).sort(([left], [right]) => left.localeCompare(right))),
    declarations: serviceTypes(`ReturnType<typeof import(${JSON.stringify(definition.api.module)}, { with: { "resolution-mode": "import" } })[${JSON.stringify(definition.api.export)}]>`, methods),
  };
}

const platformNames: Record<NativePlatform, string> = {
  ios: "iOS", macos: "macOS", android: "Android", windows: "Windows", linux: "Linux",
};

/** Where an operation is implemented, for editor documentation. */
function availability(platforms: NativePlatform[], web: boolean): string {
  const native = platforms.map(platform => platformNames[platform]).join(", ") || "none";
  return `Native: ${native}. Web: ${web ? "browser implementation" : "not supported"}.`;
}

/**
 * The native platforms that implement the plugin method `property`: those the `@platforms` tag of
 * its doc comment names, or every one the plugin builds for.
 */
function implementedOn(property: ts.Symbol, definition: PluginDefinition): NativePlatform[] {
  const tag = property.declarations?.flatMap(node => ts.getJSDocTags(node)).find(({ tagName }) => tagName.text === "platforms");
  if (!tag) return definition.platforms;
  const method = `Plugin ${definition.id}.${property.name}`;
  const names = (ts.getTextOfJSDocComment(tag.comment) ?? "").split(/\s+/).filter(Boolean);
  if (!names.length) throw new Error(`${method}: @platforms names no platform.`);
  const unknown = names.find(name => !definition.platforms.includes(name as NativePlatform));
  if (unknown) throw new Error(`${method}: @platforms names ${unknown}, which the plugin does not build for (${definition.platforms.join(", ")}).`);
  return names as NativePlatform[];
}

function browserMethods(project: ReturnType<typeof workerModule>, definition: PluginDefinition, methods: ServiceMethod[], validate: boolean): string[] {
  if (!definition.web) return [];
  const { checker } = project;
  const module = workerModule(definition.web.module, project);
  const api = module.exports.get(definition.web.export);
  if (!api || !apiObject(checker, api)) {
    throw new Error(`Plugin ${definition.id} web export must be an object implementing its frontend methods.`);
  }
  const clientModule = checker.getAmbientModules().find(symbol => symbol.name === JSON.stringify(definition.client));
  const clientSymbol = clientModule && checker.getExportsOfModule(clientModule).find(symbol => symbol.name === "TokamakClient");
  const client = clientSymbol && checker.getDeclaredTypeOfSymbol(clientSymbol);
  return api.getProperties().filter(property => !privateProperty(property)).map(property => {
    const method = methods.find(method => method.name === property.name);
    if (!method) throw new Error(`Plugin ${definition.id} web method ${property.name} has no native API method.`);
    if (validate) {
      const expected = client?.getProperty(property.name);
      const actual = checker.getTypeOfSymbolAtLocation(property, module.source);
      checkRpcMethod(checker, actual, `${definition.id}.web.${property.name}`);
      if (!expected || !checker.isTypeAssignableTo(actual, checker.getTypeOfSymbol(expected))) {
        throw new Error(`Plugin ${definition.id} web method ${property.name} must implement its generated asynchronous frontend signature.`);
      }
    }
    return property.name;
  }).sort();
}

function privateProperty(property: ts.Symbol): boolean {
  return property.declarations?.some(node => {
    const name = (node as ts.NamedDeclaration).name;
    return Boolean(ts.getCombinedModifierFlags(node) & (ts.ModifierFlags.Private | ts.ModifierFlags.Protected)) ||
      Boolean(name && ts.isPrivateIdentifier(name));
  }) ?? false;
}

function apiObject(checker: ts.TypeChecker, type: ts.Type): boolean {
  if (type.isUnionOrIntersection()) return type.types.every(member => apiObject(checker, member));
  return Boolean(type.flags & ts.TypeFlags.Object) && !type.getCallSignatures().length &&
    !type.getConstructSignatures().length && !type.getProperty("then") && !checker.getIndexInfosOfType(type).length;
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
    throw new Error(name === "default" ? `Tokamak entrypoint ${entry} must default-export a concrete class.`
      : `Tokamak entrypoint ${name} must be a concrete class.`);
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
    methods.push(methodDeclaration(checker, property, type));
  }
  methods.sort(byName);
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

function byName(left: ServiceMethod, right: ServiceMethod): number {
  return left.name < right.name ? -1 : left.name > right.name ? 1 : 0;
}

/** `property`'s declaration in generated service types, with its documentation apart from `consumed` tags. */
function methodDeclaration(checker: ts.TypeChecker, property: ts.Symbol, type: ts.Type, consumed: string[] = []): ServiceMethod {
  return {
    name: property.name,
    overloads: type.getCallSignatures().length,
    documentation: [
      ts.displayPartsToString(property.getDocumentationComment(checker)),
      ...property.getJsDocTags(checker).filter(tag => !consumed.includes(tag.name))
        .map(tag => `@${tag.name} ${ts.displayPartsToString(tag.text)}`.trim()),
    ].filter(Boolean).join("\n"),
  };
}

function applicationPrototypes(program: ts.Program, checker: ts.TypeChecker, instance: ts.Type, entry: string, allowNative = false): Set<ts.Node> | undefined {
  const module = checker.getAmbientModules().find((symbol) => symbol.name === '"cloudflare:workers"');
  const exported = module && checker.getExportsOfModule(module).find((symbol) => symbol.name === "WorkerEntrypoint");
  const base = exported && aliased(checker, exported);
  const native = allowNative ? nativeSdkExport(program, checker, entry, "TokamakEntrypoint") : undefined;
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

/** The symbol `name` exported by the core SDK module `specifier`, which `@tokamakdev/tok` uses for `entry`. */
function nativeSdkExport(program: ts.Program, checker: ts.TypeChecker, entry: string, name: string, specifier = "@tokamakdev/plugin/worker"): ts.Symbol | undefined {
  const options = program.getCompilerOptions();
  const sdk = ts.resolveModuleName("@tokamakdev/tok", entry, options, ts.sys).resolvedModule?.resolvedFileName;
  const resolved = ts.resolveModuleName(specifier, sdk ?? fileURLToPath(import.meta.url), options, ts.sys).resolvedModule;
  const source = resolved && program.getSourceFile(resolved.resolvedFileName);
  const module = source && checker.getSymbolAtLocation(source);
  const exported = module && checker.getExportsOfModule(module).find(symbol => symbol.name === name);
  return exported && aliased(checker, exported);
}

function exposedMethod(property: ts.Symbol): boolean {
  const declarations = property.declarations?.filter(ts.isMethodDeclaration) ?? [];
  if (!declarations.length || declarations.some((node) => ts.isPrivateIdentifier(node.name)) ||
    String(property.escapedName).startsWith("__@")) return false;
  if (declarations.some((node) => ts.getModifiers(node)?.some((modifier) =>
    modifier.kind === ts.SyntaxKind.PrivateKeyword || modifier.kind === ts.SyntaxKind.ProtectedKeyword))) {
    throw new Error(`Method ${property.name} is on the public JavaScript prototype; use a #private method for a local helper.`);
  }
  assertMethodName(property.name);
  return !HANDLERS.includes(property.name);
}

function assertMethodName(name: string): void {
  if (["then", "connect", "dup", "constructor", "alarm", "webSocketMessage", "webSocketClose", "webSocketError", "__tokamakRpc"].includes(name)) {
    throw new Error(`Method ${name} is reserved by the service binding contract.`);
  }
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
