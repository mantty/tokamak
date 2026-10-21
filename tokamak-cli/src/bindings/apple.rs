//! Reads the iOS and macOS SDKs' symbol graphs into a binding module.

use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use super::model::{
    Availability, Callback, Callbacks, Case, Effects, Field, Kind, Member, MemberKind, Module,
    Named, NamedKind, Param, Skipped, Ty, Type,
};
use super::swift_types;

/// An SDK the module binds against, and the target its graph is extracted for.
struct Sdk {
    platform: &'static str,
    name: &'static str,
    target: &'static str,
}

const SDKS: [Sdk; 2] = [
    Sdk {
        platform: "iOS",
        name: "iphoneos",
        target: "arm64-apple-ios15.0",
    },
    Sdk {
        platform: "macOS",
        name: "macosx",
        target: "arm64-apple-macos12.0",
    },
];

#[derive(Deserialize)]
pub(super) struct Graph {
    symbols: Vec<Symbol>,
    relationships: Vec<Relationship>,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(super) struct Symbol {
    kind: SymbolKind,
    identifier: Identifier,
    pub(super) path_components: Vec<String>,
    #[serde(default)]
    pub(super) declaration_fragments: Vec<Fragment>,
    function_signature: Option<Signature>,
    #[serde(default)]
    availability: Vec<Available>,
    swift_generics: Option<Generics>,
}

impl Symbol {
    pub(super) fn kind(&self) -> &str {
        &self.kind.identifier
    }
}

#[derive(Deserialize, Clone)]
struct SymbolKind {
    identifier: String,
}

#[derive(Deserialize, Clone)]
struct Identifier {
    precise: String,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(super) struct Fragment {
    pub(super) kind: String,
    pub(super) spelling: String,
    pub(super) precise_identifier: Option<String>,
}

#[derive(Deserialize, Clone)]
struct Signature {
    #[serde(default)]
    parameters: Vec<Parameter>,
    #[serde(default)]
    returns: Vec<Fragment>,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Parameter {
    name: String,
    internal_name: Option<String>,
    #[serde(default)]
    declaration_fragments: Vec<Fragment>,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Available {
    domain: Option<String>,
    introduced: Option<Version>,
    deprecated: Option<Version>,
    obsoleted: Option<Version>,
    message: Option<String>,
    #[serde(default)]
    is_unconditionally_unavailable: bool,
    #[serde(default)]
    is_unconditionally_deprecated: bool,
}

#[derive(Deserialize, Clone)]
struct Version {
    major: u32,
    minor: Option<u32>,
    patch: Option<u32>,
}

#[derive(Deserialize, Clone)]
struct Generics {
    #[serde(default)]
    parameters: Vec<serde_json::Value>,
}

#[derive(Deserialize, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Relationship {
    kind: String,
    source: String,
    target: String,
}

/// A symbol of the bound modules, with its availability on each platform
/// whose SDK declares it.
struct Entry {
    symbol: Symbol,
    module: String,
    platforms: Vec<(&'static str, Vec<Available>)>,
}

/// The symbols of the bound modules and their relationships. Each graph's
/// symbols are sorted by path, as extraction orders them differently each run.
pub(super) struct Index {
    entries: Vec<Entry>,
    /// Each entry by its USR and path: an Objective-C method and its `async`
    /// Swift import share a USR.
    by_symbol: HashMap<(String, Vec<String>), usize>,
    /// The first entry with each USR.
    by_usr: HashMap<String, usize>,
    by_path: HashMap<Vec<String>, usize>,
    relationships: BTreeSet<Relationship>,
}

const TYPE_KINDS: [&str; 5] = [
    "swift.class",
    "swift.struct",
    "swift.enum",
    "swift.protocol",
    "swift.typealias",
];
const ASYNC_SEQUENCE: &str = "s:Sci";
const ASYNC_ITERATOR: &str = "s:ScI";
const OPTION_SET: &str = "s:s9OptionSetP";
const ROOT_PROTOCOLS: [&str; 3] = ["c:objc(pl)NSObject", "s:s8SendableP", "s:s9AnyObjecta"];

/// Reads `modules` from the installed iOS and macOS SDKs for the plugin `id`.
pub(crate) fn read(id: &str, modules: &[String]) -> Result<Module> {
    let mut graphs = Vec::new();
    let mut versions = Vec::new();
    for sdk in &SDKS {
        versions.push(format!("{} {}", sdk.platform, sdk_version(sdk)?));
        for module in modules {
            graphs.push((sdk.platform, module.clone(), extract(sdk, module)?));
        }
    }
    let source = format!(
        "the {} SDKs: {}",
        versions.join(" and "),
        modules.join(", ")
    );
    let mut module = module(id, modules, source, graphs);
    for sdk in &SDKS {
        probe_initializers(sdk, &mut module)?;
    }
    Ok(module)
}

/// Keeps a class's `init()` on `sdk`'s platform only where Swift can call
/// it. Symbol graphs leave out the `init` that a header makes unavailable, so
/// the compiler decides.
fn probe_initializers(sdk: &Sdk, module: &mut Module) -> Result<()> {
    let mut probes = Vec::new();
    let mut lines: Vec<String> = std::iter::once("Foundation")
        .chain(module.imports.iter().map(String::as_str))
        .map(|import| format!("import {import}"))
        .collect();
    for (position, ty) in module.types.iter().enumerate() {
        let Some(member) = ty
            .members
            .iter()
            .find(|member| member.key == format!("{}.init()", ty.native))
        else {
            continue;
        };
        let Some((_, version)) = member
            .availability
            .platforms
            .iter()
            .find(|(name, _)| name == sdk.platform)
        else {
            continue;
        };
        let available = version
            .as_ref()
            .map(|version| format!("@available({} {version}, *) ", sdk.platform))
            .unwrap_or_default();
        lines.push(format!(
            "{available}func probe{position}() {{ _ = {}() }}",
            ty.native
        ));
        probes.push((lines.len(), position));
    }
    let directory = tempfile::tempdir()?;
    let file = directory.path().join("probe.swift");
    fs::write(&file, lines.join("\n"))?;
    let path = xcrun(&["--sdk", sdk.name, "--show-sdk-path"])?;
    let output = Command::new("xcrun")
        .args(["swiftc", "-typecheck", "-sdk", &path, "-target", sdk.target])
        .arg(&file)
        .output()
        .context("typecheck the initializers")?;
    let diagnostics = String::from_utf8_lossy(&output.stderr);
    for (line, position) in probes {
        if diagnostics.contains(&format!("probe.swift:{line}:")) {
            unavailable_init(module, position, sdk.platform);
        }
    }
    Ok(())
}

/// Removes `platform` from the `init()` of the type at `position`, and the
/// member once it is on no platform.
fn unavailable_init(module: &mut Module, position: usize, platform: &str) {
    let ty = &mut module.types[position];
    let key = format!("{}.init()", ty.native);
    let Some(index) = ty.members.iter().position(|member| member.key == key) else {
        return;
    };
    let platforms = &mut ty.members[index].availability.platforms;
    platforms.retain(|(name, _)| name != platform);
    if platforms.is_empty() {
        ty.members.remove(index);
        module.skipped.push(Skipped {
            symbol: key,
            reason: "init() is unavailable".to_owned(),
        });
    }
}

/// The module that `graphs`, by platform and module name, describe.
pub(super) fn module(
    id: &str,
    modules: &[String],
    source: String,
    graphs: Vec<(&'static str, String, Graph)>,
) -> Module {
    let index = Index::new(graphs);
    let mut module = Module {
        plugin: format!("{id}/apple"),
        source,
        imports: modules.to_vec(),
        ..Module::default()
    };
    Reader {
        index: &index,
        module: &mut module,
    }
    .read();
    module
}

fn sdk_version(sdk: &Sdk) -> Result<String> {
    xcrun(&["--sdk", sdk.name, "--show-sdk-version"])
}

fn xcrun(arguments: &[&str]) -> Result<String> {
    let output = Command::new("xcrun")
        .args(arguments)
        .output()
        .context("run xcrun; Apple bindings need Xcode")?;
    if !output.status.success() {
        bail!(
            "xcrun {} failed: {}",
            arguments.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// The symbol graph of `module` in `sdk`.
fn extract(sdk: &Sdk, module: &str) -> Result<Graph> {
    let output = tempfile::tempdir()?;
    let path = xcrun(&["--sdk", sdk.name, "--show-sdk-path"])?;
    let directory = output.path().to_string_lossy();
    xcrun(&[
        "swift-symbolgraph-extract",
        "-module-name",
        module,
        "-target",
        sdk.target,
        "-sdk",
        &path,
        "-minimum-access-level",
        "public",
        "-skip-synthesized-members",
        "-output-dir",
        &directory,
    ])
    .with_context(|| format!("extract {module} from the {} SDK", sdk.platform))?;
    let graph = fs::read(output.path().join(format!("{module}.symbols.json")))?;
    Ok(serde_json::from_slice(&graph)?)
}

impl Index {
    fn new(graphs: Vec<(&'static str, String, Graph)>) -> Self {
        let mut index = Self {
            entries: Vec::new(),
            by_symbol: HashMap::new(),
            by_usr: HashMap::new(),
            by_path: HashMap::new(),
            relationships: BTreeSet::new(),
        };
        for (platform, module, mut graph) in graphs {
            index.relationships.extend(graph.relationships);
            graph.symbols.sort_by(|a, b| {
                (&a.path_components, &a.identifier.precise)
                    .cmp(&(&b.path_components, &b.identifier.precise))
            });
            for symbol in graph.symbols {
                index.add(platform, &module, symbol);
            }
        }
        index
    }

    fn add(&mut self, platform: &'static str, module: &str, symbol: Symbol) {
        let key = (
            symbol.identifier.precise.clone(),
            symbol.path_components.clone(),
        );
        if let Some(&existing) = self.by_symbol.get(&key) {
            self.entries[existing]
                .platforms
                .push((platform, symbol.availability));
            return;
        }
        let position = self.entries.len();
        if TYPE_KINDS.contains(&symbol.kind.identifier.as_str()) {
            self.by_path
                .insert(symbol.path_components.clone(), position);
        }
        self.by_usr.entry(key.0.clone()).or_insert(position);
        self.by_symbol.insert(key, position);
        let availability = symbol.availability.clone();
        self.entries.push(Entry {
            symbol,
            module: module.to_owned(),
            platforms: vec![(platform, availability)],
        });
    }

    pub(super) fn symbol(&self, usr: &str) -> Option<&Symbol> {
        self.by_usr
            .get(usr)
            .map(|&position| &self.entries[position].symbol)
    }

    pub(super) fn type_at(&self, path: &[String]) -> Option<&Symbol> {
        self.by_path
            .get(path)
            .map(|&position| &self.entries[position].symbol)
    }

    pub(super) fn is_option_set(&self, symbol: &Symbol) -> bool {
        self.conforms(&symbol.identifier.precise, OPTION_SET)
    }

    pub(super) fn conforms(&self, usr: &str, protocol: &str) -> bool {
        self.related("conformsTo", usr, protocol)
    }

    fn related(&self, kind: &str, source: &str, target: &str) -> bool {
        self.relationships.contains(&Relationship {
            kind: kind.to_owned(),
            source: source.to_owned(),
            target: target.to_owned(),
        })
    }

    fn targets<'a>(&'a self, kind: &'a str, source: &'a str) -> impl Iterator<Item = &'a str> {
        self.relationships
            .iter()
            .filter(move |relationship| relationship.kind == kind && relationship.source == source)
            .map(|relationship| relationship.target.as_str())
    }

    fn sources<'a>(&'a self, kind: &'a str, target: &'a str) -> impl Iterator<Item = &'a str> {
        self.relationships
            .iter()
            .filter(move |relationship| relationship.kind == kind && relationship.target == target)
            .map(|relationship| relationship.source.as_str())
    }

    /// The protocol's requirements, with whether each is optional.
    fn requirements<'a>(&'a self, usr: &'a str) -> Vec<(&'a Entry, bool)> {
        let required = self
            .sources("requirementOf", usr)
            .map(|source| (source, false));
        let optional = self
            .sources("optionalRequirementOf", usr)
            .map(|source| (source, true));
        let mut requirements: Vec<(&Entry, bool)> = required
            .chain(optional)
            .filter_map(|(source, optional)| {
                let position = *self.by_usr.get(source)?;
                Some((&self.entries[position], optional))
            })
            .collect();
        requirements
            .sort_by_key(|(entry, _)| self.by_usr.get(&entry.symbol.identifier.precise).copied());
        requirements
    }

    /// The members of the type at `path`, in the index's order.
    fn members<'a>(&'a self, path: &'a [String]) -> impl Iterator<Item = &'a Entry> {
        self.entries.iter().filter(move |entry| {
            let components = &entry.symbol.path_components;
            components.len() == path.len() + 1 && components.starts_with(path)
        })
    }
}

/// Builds the module from the index.
struct Reader<'a> {
    index: &'a Index,
    module: &'a mut Module,
}

impl Reader<'_> {
    fn read(&mut self) {
        let index = self.index;
        for entry in &index.entries {
            let kind = entry.symbol.kind.identifier.as_str();
            if TYPE_KINDS.contains(&kind) {
                self.read_type(entry);
            } else if entry.symbol.path_components.len() == 1 {
                self.read_global(entry);
            }
        }
    }

    fn skip(&mut self, symbol: &Symbol, reason: impl Into<String>) {
        self.module.skipped.push(Skipped {
            symbol: symbol.path_components.join("."),
            reason: reason.into(),
        });
    }

    fn read_global(&mut self, entry: &Entry) {
        let Some(availability) = availability(entry, None) else {
            return self.skip(&entry.symbol, "unavailable on iOS and macOS");
        };
        match members_of(self.index, entry, availability, true) {
            Ok(members) => self.module.globals.extend(members),
            Err(reason) => self.skip(&entry.symbol, reason),
        }
    }

    fn read_type(&mut self, entry: &Entry) {
        let symbol = &entry.symbol;
        let Some(availability) = availability(entry, None) else {
            return self.skip(symbol, "unavailable on iOS and macOS");
        };
        if is_generic(symbol) {
            return self.skip(symbol, "generic types are not bound");
        }
        let usr = &symbol.identifier.precise;
        if self.index.conforms(usr, ASYNC_SEQUENCE) || self.index.conforms(usr, ASYNC_ITERATOR) {
            return self.skip(
                symbol,
                "async sequences are bound as the streams members return",
            );
        }
        let kind = match self.kind(entry) {
            Ok(kind) => kind,
            Err(reason) => return self.skip(symbol, reason),
        };
        let mut members = self.members(entry, &availability);
        match &kind {
            Kind::Protocol { callbacks: Some(callbacks) } => {
                members.retain(|member| !callbacks.methods.iter().any(|method| method.key == member.key));
            }
            Kind::Class { .. } if self.inherits_init(entry) => members.push(inherited_init(symbol, &availability)),
            Kind::Class { .. } | Kind::Protocol { .. } => {}
            // JavaScript holds values as data, so only their types' static
            // members are called; an option set's own options are its names.
            _ => members.retain(|member| {
                let own = matches!(&member.result, Ty::Named(named) if named.path == symbol.path_components);
                let constructs = member.kind == MemberKind::Initializer || (own && member.kind == MemberKind::Getter);
                member.is_static && !constructs
            }),
        }
        self.module.types.push(Type {
            path: symbol.path_components.clone(),
            native: symbol.path_components.join("."),
            runtime: runtime_name(entry),
            kind,
            availability,
            members,
            constants: Vec::new(),
        });
    }

    fn kind(&mut self, entry: &Entry) -> Result<Kind, String> {
        let symbol = &entry.symbol;
        let usr = &symbol.identifier.precise;
        match symbol.kind.identifier.as_str() {
            "swift.class" => Ok(Kind::Class {
                superclass: self.superclass(usr),
                callbacks: None,
            }),
            "swift.protocol" => Ok(Kind::Protocol {
                callbacks: self.callbacks(entry),
            }),
            "swift.enum" => cases(self.index, entry).map(|cases| Kind::Enum { cases }),
            "swift.struct" if self.index.conforms(usr, OPTION_SET) => Ok(Kind::Options {
                options: options(self.index, entry),
            }),
            "swift.struct" => fields(self.index, entry),
            _ => swift_types::alias_target(self.index, symbol).map(Kind::Alias),
        }
    }

    /// Whether an Objective-C class creates instances with the `init()` of
    /// `NSObject`: neither it nor a bound superclass declares an initializer,
    /// and its unbound superclass is `NSObject`.
    fn inherits_init(&self, entry: &Entry) -> bool {
        let mut symbol = &entry.symbol;
        loop {
            let usr = &symbol.identifier.precise;
            if !usr.starts_with("c:objc(cs)")
                || self
                    .index
                    .members(&symbol.path_components)
                    .any(|member| member.symbol.kind() == "swift.init")
            {
                return false;
            }
            let Some(superclass) = self.index.targets("inheritsFrom", usr).next() else {
                return false;
            };
            match self.index.symbol(superclass) {
                Some(bound) => symbol = bound,
                None => return superclass == "c:objc(cs)NSObject",
            }
        }
    }

    fn superclass(&self, usr: &str) -> Option<Vec<String>> {
        self.index
            .targets("inheritsFrom", usr)
            .find_map(|target| self.index.symbol(target))
            .map(|symbol| symbol.path_components.clone())
    }

    fn members(&mut self, entry: &Entry, parent: &Availability) -> Vec<Member> {
        let index = self.index;
        let actor = swift_types::text(&entry.symbol.declaration_fragments).starts_with("actor ");
        let mut members: Vec<Member> = Vec::new();
        for member_entry in index.members(&entry.symbol.path_components) {
            let Some(availability) = availability(member_entry, Some(parent)) else {
                continue;
            };
            if is_optional_requirement(index, member_entry) {
                continue;
            }
            match members_of(index, member_entry, availability, false) {
                Ok(found) => {
                    for mut member in found {
                        // Code outside an actor awaits its isolated members.
                        let declaration =
                            swift_types::text(&member_entry.symbol.declaration_fragments);
                        member.effects.asynchronous |=
                            actor && !member.is_static && !declaration.contains("nonisolated");
                        add_member(&mut members, member, &mut self.module.skipped);
                    }
                }
                Err(reason) => self.skip(&member_entry.symbol, reason),
            }
        }
        members
    }

    /// The protocol's callbacks, when JavaScript can implement all it requires.
    fn callbacks(&mut self, entry: &Entry) -> Option<Callbacks> {
        let symbol = &entry.symbol;
        let usr = &symbol.identifier.precise;
        let inherited = self
            .index
            .targets("conformsTo", usr)
            .chain(self.index.targets("inheritsFrom", usr))
            .find(|target| !ROOT_PROTOCOLS.contains(target));
        if let Some(inherited) = inherited {
            self.skip(symbol, format!("inherits requirements from {inherited}"));
            return None;
        }
        let mut methods = Vec::new();
        let index = self.index;
        for (requirement, optional) in index.requirements(usr) {
            match callback(self.index, requirement, optional) {
                Ok(Some(method)) => methods.push(method),
                Ok(None) => {}
                Err(reason) if optional => self.skip(&requirement.symbol, reason),
                Err(reason) => {
                    self.skip(symbol, format!("cannot be implemented: {reason}"));
                    return None;
                }
            }
        }
        (!methods.is_empty()).then_some(Callbacks { methods })
    }
}

/// Whether `entry` is an optional protocol requirement, which callers cannot
/// call without checking that the object implements it.
fn is_optional_requirement(index: &Index, entry: &Entry) -> bool {
    index
        .targets("optionalRequirementOf", &entry.symbol.identifier.precise)
        .next()
        .is_some()
}

/// The `init()` an Objective-C class inherits from `NSObject`.
fn inherited_init(symbol: &Symbol, availability: &Availability) -> Member {
    let path = &symbol.path_components;
    Member {
        name: "init".to_owned(),
        key: format!("{}.init()", path.join(".")),
        kind: MemberKind::Initializer,
        is_static: true,
        params: Vec::new(),
        result: Ty::Named(Named {
            path: path.clone(),
            native: path.join("."),
            kind: NamedKind::Object,
        }),
        effects: Effects::default(),
        stream: false,
        availability: availability.clone(),
        native: "init".to_owned(),
        labels: Vec::new(),
        variadic: false,
    }
}

/// Adds `member` unless another already has its name; prefers an `async`
/// declaration over a synchronous one of the same name.
fn add_member(members: &mut Vec<Member>, member: Member, skipped: &mut Vec<Skipped>) {
    let same = |other: &Member| other.name == member.name && other.is_static == member.is_static;
    match members.iter().position(same) {
        None => members.push(member),
        Some(position)
            if member.effects.asynchronous && !members[position].effects.asynchronous =>
        {
            let replaced = std::mem::replace(&mut members[position], member);
            skipped.push(Skipped {
                symbol: replaced.key,
                reason: "an async declaration has its name".to_owned(),
            });
        }
        Some(position) => skipped.push(Skipped {
            symbol: member.key,
            reason: format!("its name is taken by {}", members[position].key),
        }),
    }
}

/// The name native code reports for objects of the type: an Objective-C
/// class's or protocol's own, or a Swift type's module-qualified name.
fn runtime_name(entry: &Entry) -> String {
    let usr = &entry.symbol.identifier.precise;
    match usr
        .strip_prefix("c:objc(cs)")
        .or_else(|| usr.strip_prefix("c:objc(pl)"))
    {
        Some(name) => name.to_owned(),
        None => format!(
            "{}.{}",
            entry.module,
            entry.symbol.path_components.join(".")
        ),
    }
}

fn is_generic(symbol: &Symbol) -> bool {
    symbol
        .swift_generics
        .as_ref()
        .is_some_and(|generics| !generics.parameters.is_empty())
}

/// Where `entry` is available, within its type's `parent` availability; none
/// when on no platform.
fn availability(entry: &Entry, parent: Option<&Availability>) -> Option<Availability> {
    let mut result = Availability::default();
    for (platform, available) in &entry.platforms {
        let applies: Vec<&Available> = available
            .iter()
            .filter(|entry| {
                entry
                    .domain
                    .as_deref()
                    .is_none_or(|domain| [*platform, "*", "Swift"].contains(&domain))
            })
            .collect();
        if applies
            .iter()
            .any(|entry| entry.is_unconditionally_unavailable || entry.obsoleted.is_some())
        {
            continue;
        }
        // A member is available only where its type is, and no earlier.
        let inherited =
            match parent.map(|parent| parent.platforms.iter().find(|(name, _)| name == platform)) {
                Some(None) => continue,
                Some(Some((_, version))) => version.clone(),
                None => None,
            };
        let introduced = applies
            .iter()
            .rev()
            .find_map(|entry| entry.introduced.as_ref())
            .map(ToString::to_string);
        result
            .platforms
            .push(((*platform).to_owned(), introduced.or(inherited)));
        if result.deprecated.is_none() {
            result.deprecated = deprecation(available);
        }
    }
    (!result.platforms.is_empty()).then_some(result)
}

/// The deprecation message, when deprecated. Version 100000 marks an API
/// Apple has yet to deprecate.
fn deprecation(available: &[Available]) -> Option<String> {
    available
        .iter()
        .find(|entry| {
            let deprecated = entry
                .deprecated
                .as_ref()
                .is_some_and(|version| version.major < 100_000);
            deprecated || entry.is_unconditionally_deprecated
        })
        .map(|entry| entry.message.clone().unwrap_or_default())
}

impl std::fmt::Display for Version {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}.{}", self.major, self.minor.unwrap_or(0))?;
        match self.patch {
            Some(patch) if patch > 0 => write!(formatter, ".{patch}"),
            _ => Ok(()),
        }
    }
}

/// The members `entry` declares: a property's getter and any setter, or
/// none for a kind the binding ignores.
fn members_of(
    index: &Index,
    entry: &Entry,
    availability: Availability,
    global: bool,
) -> Result<Vec<Member>, String> {
    let Some(member) = member(index, entry, availability, global)? else {
        return Ok(Vec::new());
    };
    let declaration = swift_types::text(&entry.symbol.declaration_fragments);
    let settable = member.kind == MemberKind::Getter
        && !member.stream
        && (declaration.contains("set }")
            || (declaration.contains("var ") && !declaration.contains('{')));
    if !settable {
        return Ok(vec![member]);
    }
    let setter = Member {
        name: format!("set{}", swift_types::capitalize(&member.name)),
        key: format!("{}=", member.key),
        kind: MemberKind::Setter,
        params: vec![Param {
            name: "value".to_owned(),
            ty: member.result.clone(),
        }],
        result: Ty::Void,
        effects: Effects::default(),
        availability: member.availability.clone(),
        native: member.native.clone(),
        labels: Vec::new(),
        is_static: member.is_static,
        stream: false,
        variadic: false,
    };
    Ok(vec![member, setter])
}

/// The member `entry` declares, or none for a kind the binding ignores.
fn member(
    index: &Index,
    entry: &Entry,
    availability: Availability,
    global: bool,
) -> Result<Option<Member>, String> {
    let symbol = &entry.symbol;
    if is_generic(symbol) {
        return Err("generic members are not bound".to_owned());
    }
    let last = symbol.path_components.last().cloned().unwrap_or_default();
    let key = symbol.path_components.join(".");
    let effects = swift_types::effects(&symbol.declaration_fragments);
    let kind = symbol.kind.identifier.as_str();
    let (member_kind, is_static) = match kind {
        "swift.init" => (MemberKind::Initializer, true),
        "swift.method" => (MemberKind::Method, false),
        "swift.type.method" | "swift.func" => (MemberKind::Method, true),
        "swift.property" => (MemberKind::Getter, false),
        "swift.type.property" | "swift.var" => (MemberKind::Getter, true),
        _ => return Ok(None),
    };
    let mut member = Member {
        name: name(symbol, effects.asynchronous),
        key,
        kind: member_kind,
        is_static: is_static || global,
        params: Vec::new(),
        result: Ty::Void,
        effects,
        stream: false,
        availability,
        native: last.split('(').next().unwrap_or_default().to_owned(),
        labels: Vec::new(),
        variadic: false,
    };
    if member_kind == MemberKind::Getter {
        member.result = swift_types::property_type(index, &symbol.declaration_fragments)?;
    } else {
        signature(index, symbol, &mut member)?;
    }
    stream_result(index, &mut member);
    Ok(Some(member))
}

/// The TypeScript name of a member or callback: an Objective-C method's
/// selector, or the Swift name of a Swift member or `async` variant.
fn name(symbol: &Symbol, asynchronous: bool) -> String {
    let usr = &symbol.identifier.precise;
    let selector = ["(im)", "(cm)"]
        .iter()
        .find_map(|kind| usr.split_once(kind))
        .map(|(_, selector)| selector);
    match selector {
        Some(selector) if !asynchronous => swift_types::selector_name(selector),
        _ => swift_types::member_name(symbol.path_components.last().map_or("", String::as_str)),
    }
}

/// Reads the parameters, labels and result of a function or initializer.
fn signature(index: &Index, symbol: &Symbol, member: &mut Member) -> Result<(), String> {
    let signature = symbol.function_signature.as_ref().ok_or("no signature")?;
    member.labels = swift_types::labels(symbol.path_components.last().map_or("", String::as_str));
    for parameter in &signature.parameters {
        let ty = swift_types::parameter_type(index, &parameter.declaration_fragments)?;
        let name = parameter
            .internal_name
            .clone()
            .unwrap_or_else(|| parameter.name.clone());
        member.params.push(Param { name, ty });
    }
    member.result = if member.kind == MemberKind::Initializer {
        initializer_result(index, symbol)?
    } else {
        swift_types::parse(index, &signature.returns)?
    };
    Ok(())
}

fn initializer_result(index: &Index, symbol: &Symbol) -> Result<Ty, String> {
    let parent = &symbol.path_components[..symbol.path_components.len() - 1];
    let ty = swift_types::named(
        index,
        index
            .type_at(parent)
            .ok_or("an initializer of an unbound type")?,
    )?;
    let failable = swift_types::text(&symbol.declaration_fragments).starts_with("init?");
    Ok(if failable { ty.optional() } else { ty })
}

/// Makes a member that returns an `AsyncSequence` a stream of its elements.
fn stream_result(index: &Index, member: &mut Member) {
    let Ty::Named(named) = &member.result else {
        return;
    };
    let Some(sequence) = index.type_at(&named.path) else {
        return;
    };
    if !index.conforms(&sequence.identifier.precise, ASYNC_SEQUENCE) {
        return;
    }
    let mut element_path = named.path.clone();
    element_path.push("Element".to_owned());
    if let Some(Ok(element)) = index
        .type_at(&element_path)
        .map(|alias| swift_types::alias_target(index, alias))
    {
        member.result = element;
        member.stream = true;
    }
}

fn cases(index: &Index, entry: &Entry) -> Result<Vec<Case>, String> {
    let parent = availability(entry, None).unwrap_or_default();
    let mut cases = Vec::new();
    for case in index.members(&entry.symbol.path_components) {
        if case.symbol.kind.identifier != "swift.enum.case" {
            continue;
        }
        let declaration = swift_types::text(&case.symbol.declaration_fragments);
        if declaration.contains('(') {
            return Err("enum cases with associated values are not bound".to_owned());
        }
        let Some(availability) = availability(case, Some(&parent)) else {
            continue;
        };
        let name = case
            .symbol
            .path_components
            .last()
            .cloned()
            .unwrap_or_default();
        cases.push(Case {
            native: name.clone(),
            name,
            availability,
        });
    }
    Ok(cases)
}

/// An option set's options: its static properties of its own type.
fn options(index: &Index, entry: &Entry) -> Vec<Case> {
    let parent = availability(entry, None).unwrap_or_default();
    let own = entry
        .symbol
        .path_components
        .last()
        .cloned()
        .unwrap_or_default();
    index
        .members(&entry.symbol.path_components)
        .filter(|option| option.symbol.kind.identifier == "swift.type.property")
        .filter(|option| {
            let declaration = swift_types::text(&option.symbol.declaration_fragments);
            declaration.ends_with(&format!(": {own}"))
                || declaration.contains(&format!(": {own} {{"))
        })
        .filter_map(|option| {
            let availability = availability(option, Some(&parent))?;
            let name = option.symbol.path_components.last().cloned()?;
            Some(Case {
                native: name.clone(),
                name,
                availability,
            })
        })
        .collect()
}

/// A struct's fields: its instance properties, and whether an initializer
/// takes exactly them.
fn fields(index: &Index, entry: &Entry) -> Result<Kind, String> {
    let parent = availability(entry, None).unwrap_or_default();
    let mut fields = Vec::new();
    let mut initializers = Vec::new();
    for member in index.members(&entry.symbol.path_components) {
        let symbol = &member.symbol;
        let name = symbol.path_components.last().cloned().unwrap_or_default();
        match symbol.kind.identifier.as_str() {
            "swift.property" => {
                let Some(availability) = availability(member, Some(&parent)) else {
                    continue;
                };
                let ty = swift_types::property_type(index, &symbol.declaration_fragments)?;
                fields.push(Field {
                    name,
                    ty,
                    availability,
                });
            }
            "swift.init" => {
                let labels = swift_types::labels(&name);
                let types = symbol
                    .function_signature
                    .iter()
                    .flat_map(|signature| &signature.parameters);
                let types: Result<Vec<Ty>, String> = types
                    .map(|parameter| {
                        swift_types::parameter_type(index, &parameter.declaration_fragments)
                    })
                    .collect();
                if let Ok(types) = types {
                    initializers.push(labels.into_iter().zip(types).collect::<Vec<_>>());
                }
            }
            _ => {}
        }
    }
    let takes_fields: Vec<(Option<String>, Ty)> = fields
        .iter()
        .map(|field| (Some(field.name.clone()), field.ty.clone()))
        .collect();
    let constructible = initializers.contains(&takes_fields);
    Ok(Kind::Struct {
        fields,
        constructible,
    })
}

/// The callback a protocol requirement declares, or an error when JavaScript
/// cannot implement it.
fn callback(index: &Index, entry: &Entry, optional: bool) -> Result<Option<Callback>, String> {
    let symbol = &entry.symbol;
    if symbol.kind.identifier != "swift.method" {
        return Err(format!(
            "{} requirements are not bound",
            symbol.kind.identifier
        ));
    }
    let signature = symbol.function_signature.as_ref().ok_or("no signature")?;
    if swift_types::parse(index, &signature.returns)? != Ty::Void {
        return Err("it returns a value".to_owned());
    }
    let Some(availability) = availability(entry, None) else {
        return Ok(None);
    };
    let mut params = Vec::new();
    for parameter in &signature.parameters {
        let ty = swift_types::parameter_type(index, &parameter.declaration_fragments)?;
        let name = parameter
            .internal_name
            .clone()
            .unwrap_or_else(|| parameter.name.clone());
        params.push(Param { name, ty });
    }
    let selector = optional
        .then(|| {
            symbol
                .identifier
                .precise
                .split_once("(im)")
                .map(|(_, selector)| selector.to_owned())
        })
        .flatten();
    Ok(Some(Callback {
        key: symbol.path_components.join("."),
        name: name(symbol, false),
        params,
        declaration: swift_types::declaration(&symbol.declaration_fragments),
        default: optional,
        selector,
        availability,
    }))
}
