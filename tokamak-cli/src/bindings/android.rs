//! Reads the Android SDK's stub sources and API levels into a binding module.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use quick_xml::events::{BytesStart, Event};

use super::java::{self, Class, ClassKind, JavaMember, JavaType, MemberKind as JavaKind};
use super::model::{
    Availability, Callback, Callbacks, Case, Collection, Constant, Effects, Kind, Member,
    MemberKind, Module, Named, NamedKind, Param, Skipped, Ty, Type,
};

/// The API level that introduced a symbol, and any that deprecated it.
#[derive(Clone, Copy, Default)]
pub(super) struct Level {
    since: Option<u32>,
    deprecated: Option<u32>,
}

/// The installed Android platform the module binds against.
struct Sdk {
    level: u32,
    directory: PathBuf,
}

/// Reads the `entries`, packages or classes, from the newest installed Android
/// platform for the plugin `id`.
pub(crate) fn read(id: &str, entries: &[String]) -> Result<Module> {
    let sdk = sdk()?;
    let files = stub_files(&sdk.directory.join("android-stubs-src.jar"), entries)?;
    let versions = BufReader::new(File::open(sdk.directory.join("data/api-versions.xml"))?);
    let source = format!("the Android {} SDK: {}", sdk.level, entries.join(", "));
    module(id, source, files, &levels(versions)?)
}

/// The module that the stub `files`, by path, and the API `levels` describe.
pub(super) fn module(
    id: &str,
    source: String,
    files: Vec<(String, String)>,
    levels: &HashMap<String, Level>,
) -> Result<Module> {
    let mut classes = Vec::new();
    for (path, source) in files {
        let file = java::parse(&source).map_err(|error| anyhow::anyhow!("{path}: {error}"))?;
        for class in file.classes {
            flatten(&file.package, class, &mut classes);
        }
    }
    let mut module = Module {
        plugin: format!("{id}/android"),
        source,
        ..Module::default()
    };
    Reader {
        classes: &classes,
        levels,
        module: &mut module,
    }
    .read();
    Ok(module)
}

fn sdk() -> Result<Sdk> {
    let home = std::env::var_os("ANDROID_HOME")
        .or_else(|| std::env::var_os("ANDROID_SDK_ROOT"))
        .context("set ANDROID_HOME to the Android SDK for Android bindings")?;
    let platforms = Path::new(&home).join("platforms");
    let mut newest: Option<Sdk> = None;
    for entry in
        std::fs::read_dir(&platforms).with_context(|| format!("read {}", platforms.display()))?
    {
        let directory = entry?.path();
        let level = directory
            .file_name()
            .and_then(|name| name.to_str()?.strip_prefix("android-")?.parse::<u32>().ok());
        if let Some(level) = level
            && directory.join("android-stubs-src.jar").is_file()
            && newest.as_ref().is_none_or(|newest| level > newest.level)
        {
            newest = Some(Sdk { level, directory });
        }
    }
    newest.with_context(|| {
        format!(
            "no platform in {} has android-stubs-src.jar",
            platforms.display()
        )
    })
}

/// The stub sources of each entry: one class's file, or every file directly in a package.
fn stub_files(jar: &Path, entries: &[String]) -> Result<Vec<(String, String)>> {
    let mut archive = zip::ZipArchive::new(BufReader::new(File::open(jar)?))?;
    let names: Vec<String> = archive
        .file_names()
        .map(|name| name.map(std::borrow::Cow::into_owned))
        .collect::<Result<_, _>>()?;
    let mut files = Vec::new();
    for entry in entries {
        let directory = entry.replace('.', "/");
        let class = format!("{directory}.java");
        let selected: Vec<String> = if names.contains(&class) {
            vec![class]
        } else {
            names
                .iter()
                .filter(|name| {
                    name.strip_prefix(&format!("{directory}/"))
                        .is_some_and(|file| {
                            let source = Path::new(file)
                                .extension()
                                .is_some_and(|extension| extension.eq_ignore_ascii_case("java"));
                            source && !file.contains('/') && file != "package-info.java"
                        })
                })
                .cloned()
                .collect()
        };
        if selected.is_empty() {
            bail!("the Android SDK has no package or class {entry}");
        }
        for name in selected {
            let mut source = String::new();
            archive.by_name(&name)?.read_to_string(&mut source)?;
            files.push((name, source));
        }
    }
    Ok(files)
}

/// The API level of each class and member, by JVM name: `a/b/C`, `a/b/C#m(I)V`
/// and `a/b/C#FIELD`.
pub(super) fn levels(versions: impl std::io::BufRead) -> Result<HashMap<String, Level>> {
    let mut reader = quick_xml::Reader::from_reader(versions);
    let mut levels = HashMap::new();
    let mut class = String::new();
    let mut buffer = Vec::new();
    loop {
        match reader.read_event_into(&mut buffer)? {
            Event::Start(element) | Event::Empty(element) => {
                let level = level(&element)?;
                let name = attribute(&element, b"name")?.unwrap_or_default();
                match element.name().as_ref() {
                    b"class" => {
                        class.clone_from(&name);
                        levels.insert(name, level);
                    }
                    b"method" | b"field" => {
                        levels.insert(format!("{class}#{name}"), level);
                    }
                    _ => {}
                }
            }
            Event::Eof => return Ok(levels),
            _ => {}
        }
        buffer.clear();
    }
}

fn level(element: &BytesStart<'_>) -> Result<Level> {
    let number = |name: &[u8]| -> Result<Option<u32>> {
        Ok(attribute(element, name)?.and_then(|value| value.split('.').next()?.parse().ok()))
    };
    Ok(Level {
        since: number(b"since")?,
        deprecated: number(b"deprecated")?,
    })
}

fn attribute(element: &BytesStart<'_>, name: &[u8]) -> Result<Option<String>> {
    Ok(element
        .try_get_attribute(name)?
        .map(|attribute| {
            attribute
                .normalized_value(quick_xml::XmlVersion::Explicit1_0)
                .map(std::borrow::Cow::into_owned)
        })
        .transpose()?)
}

/// A class with its package, and the package-qualified name of each type it names.
struct Declared {
    package: String,
    class: Class,
}

impl Declared {
    /// The source name: `android.app.Notification.Builder`.
    fn source(&self) -> String {
        format!("{}.{}", self.package, self.class.path.join("."))
    }

    /// The JVM name: `android/app/Notification$Builder`.
    fn binary(&self) -> String {
        format!(
            "{}/{}",
            self.package.replace('.', "/"),
            self.class.path.join("$")
        )
    }
}

fn flatten(package: &str, mut class: Class, classes: &mut Vec<Declared>) {
    let nested = std::mem::take(&mut class.nested);
    let public = class.modifiers.iter().any(|modifier| modifier == "public");
    if public && class.kind != ClassKind::Annotation {
        classes.push(Declared {
            package: package.to_owned(),
            class,
        });
    }
    for class in nested {
        flatten(package, class, classes);
    }
}

struct Reader<'a> {
    classes: &'a [Declared],
    levels: &'a HashMap<String, Level>,
    module: &'a mut Module,
}

impl Reader<'_> {
    fn read(&mut self) {
        let bound: HashMap<String, &Declared> = self
            .classes
            .iter()
            .map(|declared| (declared.source(), declared))
            .collect();
        for declared in self.classes {
            let types = Types {
                bound: &bound,
                levels: self.levels,
                variables: declared.class.parameters.clone(),
            };
            let ty = self.read_type(declared, &types);
            self.module.types.push(ty);
        }
    }

    fn skip(&mut self, symbol: String, reason: impl Into<String>) {
        self.module.skipped.push(Skipped {
            symbol,
            reason: reason.into(),
        });
    }

    fn availability(&self, key: &str, parent: Option<&Availability>) -> Availability {
        let level = self.levels.get(key).copied().unwrap_or_default();
        let inherited = parent
            .and_then(|parent| parent.platforms.first())
            .and_then(|(_, version)| version.clone());
        let since = level.since.map(|since| since.to_string()).or(inherited);
        let mut availability = Availability {
            platforms: vec![("Android API".to_owned(), since)],
            deprecated: None,
        };
        if level.deprecated.is_some() {
            availability.deprecated = Some(String::new());
        }
        availability
    }

    fn read_type(&mut self, declared: &Declared, types: &Types<'_>) -> Type {
        let class = &declared.class;
        let mut availability = self.availability(&declared.binary(), None);
        if has(&class.annotations, "Deprecated") {
            availability.deprecated = Some(String::new());
        }
        let kind = match class.kind {
            ClassKind::Interface => Kind::Protocol {
                callbacks: self.callbacks(declared, types, &availability),
            },
            ClassKind::Enum => Kind::Enum {
                cases: cases(class, &availability),
            },
            _ => Kind::Class {
                superclass: class
                    .superclass
                    .as_ref()
                    .and_then(|superclass| types.path(&superclass.name)),
                callbacks: is_abstract(class)
                    .then(|| self.callbacks(declared, types, &availability))
                    .flatten(),
            },
        };
        let (members, constants) = self.members(declared, types, &availability);
        let generics = if class.parameters.is_empty() {
            String::new()
        } else {
            format!("<{}>", vec!["Any?"; class.parameters.len()].join(", "))
        };
        Type {
            path: class.path.clone(),
            native: declared.source() + &generics,
            runtime: declared.binary().replace('/', "."),
            kind,
            availability,
            members,
            constants,
        }
    }

    fn members(
        &mut self,
        declared: &Declared,
        types: &Types<'_>,
        parent: &Availability,
    ) -> (Vec<Member>, Vec<Constant>) {
        let mut members = Vec::new();
        let mut constants = Vec::new();
        for member in &declared.class.members {
            if !member.modifiers.iter().any(|modifier| modifier == "public") {
                continue;
            }
            let key = key(declared, member, types);
            let availability =
                self.availability(&format!("{}#{}", declared.binary(), key.1), Some(parent));
            if member.kind == JavaKind::Field
                && let Some(value) = constant(member)
            {
                constants.push(Constant {
                    name: member.name.clone(),
                    value,
                    availability,
                });
                continue;
            }
            match self::member(declared, member, types, key.0, availability) {
                Ok(found) => members.extend(found),
                Err(reason) => self.skip(format!("{}.{}", declared.source(), member.name), reason),
            }
        }
        let skipped = overloads(&mut members, types);
        self.module.skipped.extend(skipped);
        (members, constants)
    }

    /// The methods JavaScript handlers implement: an interface's, or an
    /// abstract class's overridable methods, that return nothing.
    fn callbacks(
        &mut self,
        declared: &Declared,
        types: &Types<'_>,
        parent: &Availability,
    ) -> Option<Callbacks> {
        let mut methods: Vec<Callback> = Vec::new();
        for member in &declared.class.members {
            let overridable = member.kind == JavaKind::Method
                && !member
                    .modifiers
                    .iter()
                    .any(|modifier| ["static", "final", "private"].contains(&modifier.as_str()))
                && member.ty.name == "void"
                && member.parameters.is_empty();
            if !overridable {
                continue;
            }
            let (_, descriptor) = key(declared, member, types);
            let availability =
                self.availability(&format!("{}#{descriptor}", declared.binary()), Some(parent));
            match callback(declared, member, types, availability) {
                Ok(callback) => add_callback(&mut methods, callback, &mut self.module.skipped),
                Err(reason) if !is_required(&declared.class, member) => {
                    self.skip(format!("{}.{}", declared.source(), member.name), reason);
                }
                Err(reason) => {
                    self.skip(
                        declared.source(),
                        format!("cannot be implemented: {reason}"),
                    );
                    return None;
                }
            }
        }
        (!methods.is_empty()).then_some(Callbacks { methods })
    }
}

/// The members a Java member is: a constructor, method, or a field's getter and setter.
fn member(
    declared: &Declared,
    member: &JavaMember,
    types: &Types<'_>,
    key: String,
    availability: Availability,
) -> Result<Vec<Member>, String> {
    let is_static = member.modifiers.iter().any(|modifier| modifier == "static");
    let types = types.with(&member.parameters);
    let mut params = Vec::new();
    for param in &member.params {
        let ty = types.ty(&param.ty, nullable(&param.annotations, true))?;
        let ty = if param.varargs {
            Ty::List(Box::new(ty), Collection::JavaArray)
        } else {
            ty
        };
        params.push(Param {
            name: param.name.clone(),
            ty,
        });
    }
    let variadic = member.params.last().is_some_and(|param| param.varargs);
    let base = Member {
        name: member.name.clone(),
        key,
        kind: MemberKind::Method,
        is_static,
        params,
        result: Ty::Void,
        effects: Effects {
            throws: true,
            asynchronous: false,
        },
        stream: false,
        availability,
        native: member.name.clone(),
        labels: Vec::new(),
        variadic,
    };
    match member.kind {
        JavaKind::Constructor if is_abstract(&declared.class) || is_inner(&declared.class) => {
            Ok(Vec::new())
        }
        JavaKind::Constructor => {
            let result = reference(declared);
            Ok(vec![Member {
                name: "new".to_owned(),
                kind: MemberKind::Initializer,
                is_static: true,
                result,
                ..base
            }])
        }
        JavaKind::Method => {
            let result = types.ty(&member.ty, nullable(&member.annotations, true))?;
            Ok(vec![Member { result, ..base }])
        }
        JavaKind::Field => field(member, &types, base),
    }
}

/// Keeps one of the callbacks that share a name: the one without a platform
/// default, or the first. The others keep their platform defaults.
fn add_callback(methods: &mut Vec<Callback>, callback: Callback, skipped: &mut Vec<Skipped>) {
    let Some(position) = methods
        .iter()
        .position(|method| method.name == callback.name)
    else {
        methods.push(callback);
        return;
    };
    let replaced = if methods[position].default && !callback.default {
        std::mem::replace(&mut methods[position], callback)
    } else {
        callback
    };
    skipped.push(Skipped {
        symbol: replaced.key,
        reason: "another overload of its name is implemented; it keeps its platform default"
            .to_owned(),
    });
}

fn callback(
    declared: &Declared,
    member: &JavaMember,
    types: &Types<'_>,
    availability: Availability,
) -> Result<Callback, String> {
    let mut params = Vec::new();
    let mut declarations = Vec::new();
    for param in &member.params {
        let nullable = nullable(&param.annotations, true);
        let ty = types.ty(&param.ty, nullable)?;
        declarations.push(format!(
            "{}: {}",
            kotlin_name(&param.name),
            types.kotlin(&param.ty, nullable)
        ));
        params.push(Param {
            name: param.name.clone(),
            ty,
        });
    }
    let default = !is_required(&declared.class, member);
    Ok(Callback {
        key: key(declared, member, types).0,
        name: member.name.clone(),
        params,
        declaration: format!(
            "override fun {}({})",
            kotlin_name(&member.name),
            declarations.join(", ")
        ),
        default,
        selector: None,
        availability,
    })
}

fn is_required(class: &Class, member: &JavaMember) -> bool {
    let has = |modifier: &str| member.modifiers.iter().any(|found| found == modifier);
    match class.kind {
        ClassKind::Interface => !has("default") && !has("static"),
        _ => has("abstract"),
    }
}

fn is_abstract(class: &Class) -> bool {
    class
        .modifiers
        .iter()
        .any(|modifier| modifier == "abstract")
}

/// Whether the class is an inner class, whose instances need an enclosing instance.
fn is_inner(class: &Class) -> bool {
    class.path.len() > 1
        && class.kind == ClassKind::Class
        && !class.modifiers.iter().any(|modifier| modifier == "static")
}

fn has(annotations: &[String], name: &str) -> bool {
    annotations
        .iter()
        .any(|annotation| annotation.rsplit('.').next() == Some(name))
}

/// Whether a value annotated with `annotations` may be null; `default` when unannotated.
fn nullable(annotations: &[String], default: bool) -> bool {
    if has(annotations, "NonNull") {
        false
    } else {
        has(annotations, "Nullable") || default
    }
}

fn cases(class: &Class, availability: &Availability) -> Vec<Case> {
    class
        .constants
        .iter()
        .map(|name| Case {
            name: name.clone(),
            native: name.clone(),
            availability: availability.clone(),
        })
        .collect()
}

/// A field as a getter, and as a setter unless it is final.
fn field(member: &JavaMember, types: &Types<'_>, base: Member) -> Result<Vec<Member>, String> {
    let ty = types.ty(&member.ty, nullable(&member.annotations, true))?;
    let getter = Member {
        result: ty.clone(),
        kind: MemberKind::Getter,
        ..base
    };
    if member.modifiers.iter().any(|modifier| modifier == "final") {
        return Ok(vec![getter]);
    }
    let setter = Member {
        name: format!("set{}", super::swift_types::capitalize(&getter.name)),
        key: format!("{}=", getter.key),
        kind: MemberKind::Setter,
        params: vec![Param {
            name: "value".to_owned(),
            ty,
        }],
        result: Ty::Void,
        ..getter.clone()
    };
    Ok(vec![getter, setter])
}

/// A static final field's value as a TypeScript literal.
fn constant(member: &JavaMember) -> Option<String> {
    let is = |modifier: &str| member.modifiers.iter().any(|found| found == modifier);
    if !is("static") || !is("final") {
        return None;
    }
    let value = member.value.as_ref()?;
    if value.starts_with('"') || value == "true" || value == "false" {
        return Some(value.clone());
    }
    if let Some(character) = value.strip_prefix('\'') {
        return Some(format!(
            "\"{}\"",
            character.trim_end_matches('\'').replace('"', "\\\"")
        ));
    }
    let hex = value.starts_with("0x") || value.starts_with("0X") || value.starts_with("-0x");
    let suffixes: &[char] = if hex {
        &['L', 'l']
    } else {
        &['L', 'l', 'f', 'F', 'd', 'D']
    };
    Some(value.trim_end_matches(suffixes).to_owned())
}

/// The member's key, `Outer.Inner.name(descriptor)`, and its JVM name and
/// descriptor as `api-versions.xml` lists them.
fn key(declared: &Declared, member: &JavaMember, types: &Types<'_>) -> (String, String) {
    let types = types.with(&member.parameters);
    let erase = |name: &str| types.binary(name);
    let jvm = match member.kind {
        JavaKind::Field => member.name.clone(),
        kind => {
            let name = if kind == JavaKind::Constructor {
                "<init>"
            } else {
                member.name.as_str()
            };
            let params: String = member
                .params
                .iter()
                .map(|param| {
                    let mut ty = param.ty.clone();
                    ty.dimensions += usize::from(param.varargs);
                    ty.descriptor(&erase)
                })
                .collect();
            format!("{name}({params}){}", member.ty.descriptor(&erase))
        }
    };
    (format!("{}#{jvm}", declared.class.path.join(".")), jvm)
}

/// Keeps, among the members that share a name, those TypeScript can tell
/// apart by their arguments: of each overlapping pair, the earlier.
fn overloads(members: &mut Vec<Member>, types: &Types<'_>) -> Vec<Skipped> {
    let mut skipped = Vec::new();
    let mut kept: Vec<Member> = Vec::new();
    members.sort_by_key(|member| {
        member
            .availability
            .platforms
            .first()
            .and_then(|(_, version)| version.as_ref()?.parse::<u32>().ok())
    });
    for member in members.drain(..) {
        let overlapping = kept.iter().find(|other| {
            other.name == member.name
                && other.is_static == member.is_static
                && other.params.len() == member.params.len()
                && other
                    .params
                    .iter()
                    .zip(&member.params)
                    .all(|(left, right)| types.overlap(&left.ty, &right.ty))
        });
        match overlapping {
            Some(other) => skipped.push(Skipped {
                symbol: member.key,
                reason: format!("TypeScript cannot tell its arguments from {}'s", other.key),
            }),
            None => kept.push(member),
        }
    }
    *members = kept;
    skipped
}

/// Maps Java types to binding types for one declaration.
struct Types<'a> {
    bound: &'a HashMap<String, &'a Declared>,
    /// The API levels, whose keys name every SDK class.
    levels: &'a HashMap<String, Level>,
    /// The type variables in scope, which erase to `Object`.
    variables: Vec<String>,
}

impl Types<'_> {
    fn with(&self, variables: &[String]) -> Types<'_> {
        Types {
            bound: self.bound,
            levels: self.levels,
            variables: self.variables.iter().chain(variables).cloned().collect(),
        }
    }

    /// The bound type at the source name, by path.
    fn path(&self, name: &str) -> Option<Vec<String>> {
        self.bound
            .get(name)
            .map(|declared| declared.class.path.clone())
    }

    /// The JVM name of a source type name, `.`-separated: a type variable's
    /// erasure, or the SDK class it names, with `$` before nested classes.
    fn binary(&self, name: &str) -> Option<String> {
        if self.variables.iter().any(|variable| variable == name) {
            return Some("java.lang.Object".to_owned());
        }
        let parts: Vec<&str> = name.split('.').collect();
        (1..=parts.len()).rev().find_map(|split| {
            let candidate = format!("{}${}", parts[..split].join("/"), parts[split..].join("$"));
            let candidate = candidate.trim_end_matches('$');
            self.levels
                .contains_key(candidate)
                .then(|| candidate.replace('/', "."))
        })
    }

    fn ty(&self, ty: &JavaType, nullable: bool) -> Result<Ty, String> {
        let element = JavaType {
            dimensions: 0,
            ..ty.clone()
        };
        if ty.dimensions > 0 {
            if ty.dimensions == 1 && ty.name == "byte" {
                return Ok(optional(Ty::Bytes, nullable));
            }
            let element = self.ty(
                &JavaType {
                    dimensions: ty.dimensions - 1,
                    ..ty.clone()
                },
                true,
            )?;
            return Ok(optional(
                Ty::List(Box::new(element), Collection::JavaArray),
                nullable,
            ));
        }
        let primitive = match element.name.as_str() {
            "void" => Some(Ty::Void),
            "boolean" => Some(Ty::Bool),
            "byte" | "short" | "int" | "long" => Some(Ty::Int(element.name.clone())),
            "float" | "double" => Some(Ty::Float(element.name.clone())),
            "char" => Some(Ty::Char),
            _ => None,
        };
        if let Some(primitive) = primitive {
            return Ok(primitive);
        }
        Ok(optional(self.object(&element)?, nullable))
    }

    fn object(&self, ty: &JavaType) -> Result<Ty, String> {
        let argument = |position: usize| -> Result<Ty, String> {
            ty.arguments
                .get(position)
                .map_or(Ok(Ty::Any), |argument| self.ty(argument, true))
        };
        Ok(match ty.name.as_str() {
            name if self.variables.iter().any(|variable| variable == name) => Ty::Any,
            "java.lang.Object" => Ty::Any,
            "java.lang.String" | "java.lang.CharSequence" => Ty::String(ty.name.clone()),
            "java.lang.Boolean" => Ty::Bool,
            "java.lang.Byte" | "java.lang.Short" | "java.lang.Integer" | "java.lang.Long" => {
                Ty::Int(boxed(&ty.name))
            }
            "java.lang.Float" | "java.lang.Double" => Ty::Float(boxed(&ty.name)),
            "java.util.List" | "java.util.Collection" => {
                Ty::List(Box::new(argument(0)?), Collection::Array)
            }
            "java.util.Set" => Ty::List(Box::new(argument(0)?), Collection::Set),
            "java.util.Map"
                if ty
                    .arguments
                    .first()
                    .is_some_and(|key| key.name == "java.lang.String") =>
            {
                Ty::Record(Box::new(argument(1)?))
            }
            name => {
                let declared = self
                    .bound
                    .get(name)
                    .ok_or_else(|| format!("{name} is not bound"))?;
                let Ty::Named(mut named) = reference(declared) else {
                    return Err(format!("{name} is not bound"));
                };
                if !ty.arguments.is_empty() {
                    let arguments: Vec<String> = ty
                        .arguments
                        .iter()
                        .map(|argument| self.kotlin(argument, true))
                        .collect();
                    named.native = format!("{}<{}>", named.native, arguments.join(", "));
                }
                Ty::Named(named)
            }
        })
    }

    /// The Kotlin spelling of a Java type, for an override's parameter.
    fn kotlin(&self, ty: &JavaType, nullable: bool) -> String {
        let question = if nullable { "?" } else { "" };
        if ty.dimensions > 0 {
            let element = JavaType {
                dimensions: ty.dimensions - 1,
                ..ty.clone()
            };
            let array = match (ty.dimensions, element.name.as_str()) {
                (
                    1,
                    primitive @ ("boolean" | "byte" | "char" | "short" | "int" | "long" | "float"
                    | "double"),
                ) => {
                    format!("{}Array", super::swift_types::capitalize(primitive))
                }
                _ => format!("Array<{}>", self.kotlin(&element, true)),
            };
            return format!("{array}{question}");
        }
        let name = match ty.name.as_str() {
            "boolean" => return "Boolean".to_owned(),
            primitive @ ("byte" | "char" | "short" | "int" | "long" | "float" | "double") => {
                return super::swift_types::capitalize(primitive);
            }
            name if self.variables.iter().any(|variable| variable == name) => {
                return "Any?".to_owned();
            }
            "java.lang.Boolean" => return "Boolean?".to_owned(),
            "java.lang.Byte" | "java.lang.Short" | "java.lang.Long" | "java.lang.Float"
            | "java.lang.Double" => return format!("{}?", &ty.name["java.lang.".len()..]),
            "java.lang.Integer" => return "Int?".to_owned(),
            "java.lang.Character" => return "Char?".to_owned(),
            name => KOTLIN_TYPES
                .iter()
                .find(|(java, _)| *java == name)
                .map_or(name, |(_, kotlin)| kotlin)
                .to_owned(),
        };
        if ty.arguments.is_empty() {
            return format!("{name}{question}");
        }
        let arguments: Vec<String> = ty
            .arguments
            .iter()
            .map(|argument| self.kotlin(argument, true))
            .collect();
        format!("{name}<{}>{question}", arguments.join(", "))
    }

    /// Whether one JavaScript argument could be a value of either type.
    fn overlap(&self, left: &Ty, right: &Ty) -> bool {
        match (left, right) {
            (Ty::Optional(left), right) | (right, Ty::Optional(left)) => self.overlap(left, right),
            (Ty::Any, _) | (_, Ty::Any) => true,
            (Ty::Named(left), Ty::Named(right)) => {
                self.related(&left.path, &right.path) || self.related(&right.path, &left.path)
            }
            (left, right) => category(left) == category(right),
        }
    }

    /// Whether the class at `path` is, or derives from, the class at `ancestor`.
    fn related(&self, path: &[String], ancestor: &[String]) -> bool {
        let mut seen = HashSet::new();
        let mut current = self
            .bound
            .values()
            .copied()
            .find(|declared| declared.class.path == path);
        while let Some(declared) = current {
            if declared.class.path == ancestor || !seen.insert(declared.source()) {
                return declared.class.path == ancestor;
            }
            let supertypes = declared
                .class
                .superclass
                .iter()
                .chain(&declared.class.interfaces);
            if supertypes
                .clone()
                .any(|supertype| self.path(&supertype.name).as_deref() == Some(ancestor))
            {
                return true;
            }
            current = declared
                .class
                .superclass
                .as_ref()
                .and_then(|superclass| self.bound.get(&superclass.name).copied());
        }
        false
    }
}

/// A reference to the declared class.
fn reference(declared: &Declared) -> Ty {
    let kind = if declared.class.kind == ClassKind::Enum {
        NamedKind::Enum
    } else {
        NamedKind::Object
    };
    Ty::Named(Named {
        path: declared.class.path.clone(),
        native: declared.source(),
        kind,
    })
}

/// The JavaScript kind of a value of `ty`.
fn category(ty: &Ty) -> &'static str {
    match ty {
        Ty::Bool => "boolean",
        Ty::Int(_) | Ty::Float(_) | Ty::Number(_) => "number",
        Ty::Char | Ty::String(_) => "string",
        Ty::Bytes => "bytes",
        Ty::List(..) => "array",
        Ty::Record(_) => "object",
        Ty::Named(named) if named.kind == NamedKind::Enum => "string",
        _ => "other",
    }
}

fn optional(ty: Ty, nullable: bool) -> Ty {
    if nullable { ty.optional() } else { ty }
}

/// The primitive a boxed Java type holds, which Kotlin decodes it as.
fn boxed(name: &str) -> String {
    match name {
        "java.lang.Integer" => "int".to_owned(),
        name => name.trim_start_matches("java.lang.").to_lowercase(),
    }
}

/// The Java types Kotlin source names by its own types.
const KOTLIN_TYPES: [(&str, &str); 13] = [
    ("java.lang.Object", "Any"),
    ("java.lang.String", "String"),
    ("java.lang.CharSequence", "CharSequence"),
    ("java.lang.Number", "Number"),
    ("java.lang.Throwable", "Throwable"),
    ("java.lang.Comparable", "Comparable"),
    ("java.lang.Enum", "Enum"),
    ("java.lang.Iterable", "MutableIterable"),
    ("java.util.Iterator", "MutableIterator"),
    ("java.util.Collection", "MutableCollection"),
    ("java.util.List", "MutableList"),
    ("java.util.Set", "MutableSet"),
    ("java.util.Map", "MutableMap"),
];

const KOTLIN_KEYWORDS: [&str; 10] = [
    "as",
    "fun",
    "in",
    "interface",
    "is",
    "object",
    "typealias",
    "typeof",
    "val",
    "when",
];

/// A name as Kotlin source spells it, quoting a keyword.
pub(super) fn kotlin_name(name: &str) -> String {
    if KOTLIN_KEYWORDS.contains(&name) {
        format!("`{name}`")
    } else {
        name.to_owned()
    }
}
