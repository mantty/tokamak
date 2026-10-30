//! The description of a plugin's native methods on one platform family, which the family's
//! platform pack writes, and the TypeScript platform API generated from it.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use clap::ValueEnum;
use serde::Deserialize;
use tokamak_cli::Platform;
use walkdir::WalkDir;

use super::packs::PlatformPack;
use super::plugins;

/// A plugin's exported native methods and events on one platform family, and the types they use.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Description {
    methods: Vec<Method>,
    #[serde(default)]
    events: Vec<Event>,
    #[serde(default)]
    types: Vec<NamedType>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Method {
    name: String,
    kind: Kind,
    #[serde(default)]
    doc: String,
    #[serde(default)]
    parameters: Vec<Field>,
    /// A call's result or a stream's values; a resource has none.
    returns: Option<Type>,
    #[serde(default)]
    overflow: Overflow,
    /// The family's platforms that have the method, when only some of them do.
    #[serde(default)]
    platforms: Vec<String>,
}

/// An event native code emits, which the Worker's listeners receive and may reply to.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Event {
    name: String,
    #[serde(default)]
    doc: String,
    #[serde(rename = "type")]
    ty: Type,
    /// What a listener replies with, for an event that takes a reply.
    reply: Option<Type>,
    /// The family's platforms that declare the event's type, when only some of them do.
    #[serde(default)]
    platforms: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
enum Kind {
    Call,
    Stream,
    Resource,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
enum Overflow {
    #[default]
    Error,
    DropOldest,
}

/// A named value: a parameter or a struct field.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Field {
    name: String,
    #[serde(rename = "type")]
    ty: Type,
}

#[derive(Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
enum Type {
    String,
    Number,
    Boolean,
    Void,
    Bytes,
    Body,
    Reference,
    Json,
    Optional(Box<Type>),
    Array(Box<Type>),
    Record(Box<Type>),
    Named(String),
}

/// A struct, with its fields; a string enum, with its values; or a union, with the forms its
/// values take and, when the forms are objects that name theirs, the tag field that does.
#[derive(Debug, Deserialize)]
struct NamedType {
    name: String,
    #[serde(flatten)]
    shape: Shape,
    tag: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
enum Shape {
    Fields(Vec<Field>),
    Values(Vec<String>),
    Alternatives(Vec<Alternative>),
}

/// A form of a union's values: its name, as a string or a tag, or a value of its type.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Alternative {
    name: String,
    #[serde(rename = "type")]
    ty: Option<Type>,
}

/// A family of platforms that share a plugin's native code, in the package directory of its
/// name, which holds one registration file, listing the methods the Worker calls and the events
/// it receives.
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub(crate) enum Family {
    Apple,
    Android,
}

impl Family {
    pub(crate) const ALL: [Self; 2] = [Self::Apple, Self::Android];

    pub(crate) fn name(self) -> &'static str {
        self.platform().family()
    }

    /// The platform whose pack describes the family's plugins.
    fn platform(self) -> Platform {
        match self {
            Self::Apple => Platform::Macos,
            Self::Android => Platform::Android,
        }
    }

    pub(crate) fn platforms(self) -> &'static [Platform] {
        match self {
            Self::Apple => &[Platform::Ios, Platform::Macos],
            Self::Android => &[Platform::Android],
        }
    }

    /// The registration file's name.
    pub(crate) fn registration(self) -> &'static str {
        match self {
            Self::Apple => "Exports.swift",
            Self::Android => "Exports.kt",
        }
    }

    /// The generated TypeScript API's path in the plugin package.
    pub(crate) fn api(self) -> &'static str {
        match self {
            Self::Apple => "src/apple.generated.ts",
            Self::Android => "src/android.generated.ts",
        }
    }
}

/// Writes `family`'s TypeScript API and native dispatch into the plugin package at `root`,
/// generated from the package's registration file.
pub(crate) fn generate(root: &Path, family: Family) -> Result<()> {
    let files = generated(root, family)?.map_err(anyhow::Error::msg)?;
    for (path, contents) in &files {
        let path = root.join(path);
        if let Some(directory) = path.parent() {
            fs::create_dir_all(directory)?;
        }
        fs::write(&path, contents).with_context(|| format!("Write {}", path.display()))?;
    }
    let paths = files
        .iter()
        .map(|(path, _)| path.as_str())
        .collect::<Vec<_>>();
    println!("Generated {}", paths.join(", "));
    Ok(())
}

/// Fails when the files generated from the registration files of the plugin package at `root`
/// differ from what this host generates. Families this host cannot describe are not checked.
pub(crate) fn check(root: &Path) -> Result<()> {
    let mut stale = Vec::new();
    for family in Family::ALL {
        if registration(root, family)?.is_none() {
            continue;
        }
        let Ok(files) = generated(root, family)?.inspect_err(|reason| {
            eprintln!("{} files are not checked: {reason}", family.name());
        }) else {
            continue;
        };
        let changed = files
            .into_iter()
            .filter(|(path, contents)| fs::read(root.join(path)).ok().as_ref() != Some(contents))
            .map(|(path, _)| path)
            .collect::<Vec<_>>();
        if !changed.is_empty() {
            stale.push(format!(
                "`tok plugin generate {}` changes {}",
                family.name(),
                changed.join(", ")
            ));
        }
    }
    if !stale.is_empty() {
        bail!("generated files are out of date:\n{}", stale.join("\n"));
    }
    Ok(())
}

/// Generated files' contents, by path in the plugin package.
type Files = Vec<(String, Vec<u8>)>;

/// The files `family`'s generation writes into the plugin package at `root`, or why this host
/// cannot generate them.
fn generated(root: &Path, family: Family) -> Result<Result<Files, String>> {
    let plugin = plugins::package(root)?;
    let platforms = family
        .platforms()
        .iter()
        .copied()
        .filter(|platform| plugin.builds_for(platform.namespace()))
        .collect::<Vec<_>>();
    if platforms.is_empty() {
        bail!("tokamak-plugin.json has no {} platform", family.name());
    }
    let Some(registration) = registration(root, family)? else {
        bail!(
            "{} has no {}: it lists the native methods the Worker calls and the events it receives",
            family.name(),
            family.registration()
        );
    };
    let pack = match PlatformPack::installed(family.platform())? {
        Ok(pack) => pack,
        Err(missing) => return Ok(Err(missing)),
    };
    let output = tempfile::tempdir()?;
    let namespaces = platforms
        .iter()
        .map(|platform| platform.namespace())
        .collect::<Vec<_>>();
    if !pack.describe(root, &root.join(&registration), output.path(), &namespaces)? {
        return Ok(Err(format!(
            "this host cannot describe {} plugins",
            family.name()
        )));
    }
    let description: Description =
        serde_json::from_slice(&fs::read(output.path().join("description.json"))?)
            .context("the platform pack wrote an invalid description")?;
    description
        .validate()
        .with_context(|| format!("{registration} cannot be called from TypeScript"))?;
    let mut files = vec![(
        family.api().to_owned(),
        typescript(&description, &plugin.id, family, &platforms).into_bytes(),
    )];
    // The native files go beside the registration file.
    let directory = registration
        .rsplit_once('/')
        .map_or("", |(directory, _)| directory);
    for entry in fs::read_dir(output.path())? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name != "description.json" {
            files.push((format!("{directory}/{name}"), fs::read(entry.path())?));
        }
    }
    files.sort();
    Ok(Ok(files))
}

/// The path, in the plugin package at `root`, of the registration file `family`'s directory
/// holds outside its build outputs, if it holds one.
fn registration(root: &Path, family: Family) -> Result<Option<String>> {
    let directory = root.join(family.name());
    if !directory.is_dir() {
        return Ok(None);
    }
    let entries = WalkDir::new(&directory)
        .min_depth(1)
        .into_iter()
        .filter_entry(|entry| {
            !entry
                .path()
                .strip_prefix(&directory)
                .is_ok_and(plugins::is_build_output)
        });
    let mut found = Vec::new();
    for entry in entries {
        let entry = entry?;
        if entry.file_type().is_file() && entry.file_name() == family.registration() {
            let path = entry.path().strip_prefix(root)?;
            found.push(
                path.components()
                    .map(|part| part.as_os_str().to_string_lossy())
                    .collect::<Vec<_>>()
                    .join("/"),
            );
        }
    }
    found.sort();
    match found.as_slice() {
        [] => Ok(None),
        [path] => Ok(Some(path.clone())),
        paths => bail!(
            "{} has more than one {}: {}",
            family.name(),
            family.registration(),
            paths.join(", ")
        ),
    }
}

/// Names the generated API declares or imports, which a described type cannot have.
const GENERATED: &[&str] = &[
    "Android",
    "Apple",
    "JsonStream",
    "JsonValue",
    "NativeEndpoint",
    "NativeReference",
    "TokamakContext",
    "decodeJson",
    "defineEvent",
    "encodeJson",
];

/// Words a parameter cannot be named: those a module's strict mode reserves, and the names
/// generated methods use.
const RESERVED: &[&str] = &[
    "arguments",
    "await",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "decodeJson",
    "default",
    "delete",
    "do",
    "else",
    "encodeJson",
    "endpoint",
    "enum",
    "eval",
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "function",
    "if",
    "implements",
    "import",
    "in",
    "instanceof",
    "interface",
    "let",
    "new",
    "null",
    "package",
    "private",
    "protected",
    "public",
    "return",
    "static",
    "super",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typeof",
    "var",
    "void",
    "while",
    "with",
    "yield",
];

impl Description {
    /// Checks the rules every family's methods follow.
    pub(crate) fn validate(&self) -> Result<()> {
        let types = self.described_types()?;
        let mut methods = BTreeSet::new();
        for method in &self.methods {
            if !methods.insert(method.name.as_str()) {
                bail!("method {} is exported twice", method.name);
            }
            method
                .validate(&types)
                .map_err(|error| error.context(format!("method {}", method.name)))?;
        }
        let mut listeners = BTreeSet::new();
        for event in &self.events {
            check_identifier("event", &event.name)?;
            if !listeners.insert(event.listener()) {
                bail!("two events register {}", event.listener());
            }
            event
                .validate(&types)
                .map_err(|error| error.context(format!("event {}", event.name)))?;
        }
        Ok(())
    }

    /// The described types, by name, once each one's name and shape are checked.
    fn described_types(&self) -> Result<Types<'_>> {
        let mut types = Types::new();
        for named in &self.types {
            check_identifier("type", &named.name)?;
            if GENERATED.contains(&named.name.as_str()) {
                bail!("type {} has a name the generated API declares", named.name);
            }
            if types.insert(named.name.as_str(), named).is_some() {
                bail!("type {} is described twice", named.name);
            }
        }
        for named in &self.types {
            named.validate(&types)?;
        }
        Ok(types)
    }
}

impl Event {
    fn validate(&self, types: &Types) -> Result<()> {
        check_value(&self.ty, types)?;
        match &self.reply {
            Some(reply) => check_value(reply, types).context("reply"),
            None => Ok(()),
        }
    }

    /// The name of the function that registers the event's listeners, such as `onUpdate`.
    fn listener(&self) -> String {
        let mut listener = String::from("on");
        let mut characters = self.name.chars();
        listener.extend(characters.next().map(|first| first.to_ascii_uppercase()));
        listener.push_str(characters.as_str());
        listener
    }
}

/// The described types, by name.
type Types<'a> = BTreeMap<&'a str, &'a NamedType>;

impl NamedType {
    fn validate(&self, types: &Types) -> Result<()> {
        let fields = match (&self.shape, &self.tag) {
            (Shape::Alternatives(alternatives), tag) => {
                return self.validate_union(alternatives, tag.as_deref(), types);
            }
            (_, Some(_)) => bail!("{} has a tag but is not a union", self.name),
            (Shape::Fields(fields), None) => fields,
            (Shape::Values(values), None) if values.is_empty() => {
                bail!("enum {} has no cases", self.name)
            }
            (Shape::Values(values), None) => {
                let mut cases = BTreeSet::new();
                if let Some(value) = values.iter().find(|value| !cases.insert(value.as_str())) {
                    bail!("{} has two cases named {value}", self.name);
                }
                return Ok(());
            }
        };
        let mut names = BTreeSet::new();
        for field in fields {
            check_identifier("field", &field.name)?;
            if !names.insert(field.name.as_str()) {
                bail!("{} has two fields named {}", self.name, field.name);
            }
            check_value(&field.ty, types)
                .map_err(|error| error.context(format!("{}.{}", self.name, field.name)))?;
        }
        Ok(())
    }

    /// Checks that the union's forms can be told apart: by the tag field, or else by their JSON.
    fn validate_union(
        &self,
        alternatives: &[Alternative],
        tag: Option<&str>,
        types: &Types,
    ) -> Result<()> {
        if alternatives.is_empty() {
            bail!("union {} has no forms", self.name);
        }
        let mut names = BTreeSet::new();
        let mut forms = Vec::new();
        for alternative in alternatives {
            check_identifier("form", &alternative.name)?;
            if !names.insert(alternative.name.as_str()) {
                bail!("{} has two forms named {}", self.name, alternative.name);
            }
            let context = || format!("{}.{}", self.name, alternative.name);
            forms.push(form(alternative, types).with_context(context)?);
        }
        match tag {
            Some(tag) => check_tagged(&self.name, tag, &forms),
            None => check_untagged(&self.name, &forms),
        }
    }
}

/// What a union's form is in JSON.
enum Form<'a> {
    /// The string of the form's name.
    Name(&'a str),
    /// A string enum's values.
    Values(&'a [String]),
    /// Any other value of the kind.
    Kind(JsonKind),
    /// An object with the fields of the struct of the name.
    Struct(&'a str, &'a [Field]),
}

#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
enum JsonKind {
    String,
    Number,
    Boolean,
    Array,
    Object,
}

fn form<'a>(alternative: &'a Alternative, types: &Types<'a>) -> Result<Form<'a>> {
    let Some(ty) = &alternative.ty else {
        return Ok(Form::Name(&alternative.name));
    };
    check_value(ty, types)?;
    match ty {
        Type::String => Ok(Form::Kind(JsonKind::String)),
        Type::Number => Ok(Form::Kind(JsonKind::Number)),
        Type::Boolean => Ok(Form::Kind(JsonKind::Boolean)),
        Type::Array(_) => Ok(Form::Kind(JsonKind::Array)),
        Type::Record(_) => Ok(Form::Kind(JsonKind::Object)),
        Type::Named(name) => match &types[name.as_str()].shape {
            Shape::Values(values) => Ok(Form::Values(values)),
            Shape::Fields(fields) => Ok(Form::Struct(name, fields)),
            Shape::Alternatives(_) => Err(anyhow!("a union's form is not a union")),
        },
        Type::Optional(_) => Err(anyhow!("a union's form is not optional")),
        _ => Err(anyhow!("a union's form is not any JSON value")),
    }
}

/// Checks that each form is a name or a struct without the field `tag`.
fn check_tagged(union: &str, tag: &str, forms: &[Form]) -> Result<()> {
    check_identifier("tag", tag)?;
    for form in forms {
        match form {
            Form::Struct(name, fields) if fields.iter().any(|field| field.name == tag) => {
                bail!("{name} has the field {tag}, which tags {union}'s forms")
            }
            Form::Name(_) | Form::Struct(..) => {}
            Form::Values(_) | Form::Kind(_) => {
                bail!("each form of {union}, which {tag} tags, is a name or a struct")
            }
        }
    }
    Ok(())
}

/// Checks that the forms differ in kind, strings in value, and objects in a required field.
fn check_untagged(union: &str, forms: &[Form]) -> Result<()> {
    let mut kinds = BTreeSet::new();
    let mut strings = BTreeSet::new();
    let mut structs = Vec::new();
    for form in forms {
        let distinct = match form {
            Form::Name(name) => strings.insert(*name),
            Form::Values(values) => values.iter().all(|value| strings.insert(value)),
            Form::Kind(kind) => kinds.insert(*kind),
            Form::Struct(name, fields) => {
                structs.push((*name, *fields));
                true
            }
        };
        if !distinct {
            bail!("two of {union}'s forms are the same JSON");
        }
    }
    if kinds.contains(&JsonKind::String) && !strings.is_empty() {
        bail!("{union}'s string form takes the strings of its other forms");
    }
    if kinds.contains(&JsonKind::Object) && !structs.is_empty() {
        bail!("{union}'s object form takes the objects of its struct forms");
    }
    if structs.len() < 2 {
        return Ok(());
    }
    // A struct form's key is a required field that no other struct form has.
    let mut counts = BTreeMap::<&str, usize>::new();
    for field in structs.iter().flat_map(|(_, fields)| *fields) {
        *counts.entry(field.name.as_str()).or_default() += 1;
    }
    let keyed = |fields: &[Field]| {
        fields
            .iter()
            .any(|field| !matches!(field.ty, Type::Optional(_)) && counts[field.name.as_str()] == 1)
    };
    if let Some((name, _)) = structs.iter().find(|(_, fields)| !keyed(fields)) {
        bail!("{name} has no required field that {union}'s other object forms lack");
    }
    Ok(())
}

impl Method {
    fn validate(&self, types: &Types) -> Result<()> {
        check_identifier("method", &self.name)?;
        if self.name == "then" {
            bail!("a method named then makes the platform API a promise");
        }
        let mut names = BTreeSet::new();
        for (index, parameter) in self.parameters.iter().enumerate() {
            check_identifier("parameter", &parameter.name)?;
            if RESERVED.contains(&parameter.name.as_str()) {
                bail!("parameter {} has a reserved name", parameter.name);
            }
            if !names.insert(parameter.name.as_str()) {
                bail!("two parameters are named {}", parameter.name);
            }
            self.check_parameter(index, &parameter.ty, types)
                .map_err(|error| error.context(format!("parameter {}", parameter.name)))?;
        }
        match (self.kind, &self.returns) {
            (Kind::Call, Some(Type::Void | Type::Bytes)) | (Kind::Resource, None) => Ok(()),
            (Kind::Call | Kind::Stream, Some(returns)) => check_value(returns, types),
            (Kind::Resource, Some(_)) => bail!("a resource returns a native object"),
            (Kind::Call | Kind::Stream, None) => bail!("the result type is missing"),
        }
    }

    fn check_parameter(&self, index: usize, ty: &Type, types: &Types) -> Result<()> {
        let last = index + 1 == self.parameters.len();
        match ty {
            Type::Reference => Ok(()),
            Type::Body if self.kind == Kind::Call && last => Ok(()),
            Type::Body => bail!("a body is the last parameter of a call"),
            Type::Bytes if self.kind == Kind::Call && self.parameters.len() == 1 => Ok(()),
            Type::Bytes => bail!("bytes are a call's only parameter or its result"),
            ty => check_value(ty, types),
        }
    }
}

/// Checks that `ty` is JSON: a string, number, boolean, any JSON value or described type, or an
/// optional, array or record of one.
fn check_value(ty: &Type, types: &Types) -> Result<()> {
    match ty {
        Type::String | Type::Number | Type::Boolean | Type::Json => Ok(()),
        Type::Optional(inner) if matches!(**inner, Type::Optional(_)) => {
            bail!("an optional of an optional is indistinguishable from it in JSON")
        }
        Type::Optional(inner) | Type::Array(inner) | Type::Record(inner) => {
            check_value(inner, types)
        }
        Type::Named(name) if types.contains_key(name.as_str()) => Ok(()),
        Type::Named(name) => bail!("type {name} is not described"),
        Type::Void => bail!("void is only a call's result"),
        Type::Bytes => bail!("bytes are a call's only parameter or its result"),
        Type::Body => bail!("a body is the last parameter of a call"),
        Type::Reference => bail!("a native reference is a parameter"),
    }
}

fn check_identifier(kind: &str, name: &str) -> Result<()> {
    let mut characters = name.chars();
    let valid = characters
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_' || first == '$')
        && characters
            .all(|character| character.is_ascii_alphanumeric() || "_$".contains(character));
    if !valid {
        bail!("{kind} name {name:?} is not a JavaScript identifier");
    }
    Ok(())
}

/// The TypeScript platform API of plugin `id` on the `platforms` of `family` it supports: the
/// described types, an interface of the methods, the factory that creates it, and the functions
/// that register listeners of the events.
pub(crate) fn typescript(
    description: &Description,
    id: &str,
    family: Family,
    platforms: &[Platform],
) -> String {
    let names = platforms
        .iter()
        .map(|platform| platform.display_name())
        .collect::<Vec<_>>()
        .join(" and ");
    let interface = interface_name(family);
    let mut source = format!(
        "// Generated by `tok plugin generate {}`. Do not edit.\n{}",
        family.name(),
        imports(description)
    );
    for named in &description.types {
        source.push('\n');
        source.push_str(&type_declaration(named));
    }
    let _ = write!(
        source,
        "\n/** The {id} plugin's native methods on {names}. */\nexport interface {interface} {{\n"
    );
    source.extend(description.methods.iter().map(signature));
    let namespaces = platforms
        .iter()
        .map(|platform| platform.namespace())
        .collect::<Vec<_>>();
    let _ = write!(
        source,
        "}}\n\n/**\n * The {id} plugin's native methods. Off {names} they reject with `NotSupportedError`.\n * Aborting `signal` cancels their calls, streams and resource acquisitions.\n */\nexport function {}(ctx: TokamakContext, signal?: AbortSignal): {interface} {{\n  const endpoint = new NativeEndpoint(ctx, {id:?}, {namespaces:?}, signal);\n  return {{\n",
        family.name()
    );
    source.extend(
        description
            .methods
            .iter()
            .map(|method| format!("    {}: {},\n", method.name, implementation(method))),
    );
    source.push_str("  };\n}\n");
    if !description.events.is_empty() {
        source.push_str(&events(description, id, family, &namespaces));
    }
    source
}

/// The declarations of the events' data and replies, which the Worker's listeners receive as
/// `<id>.<family>.<name>`, and the functions that register the listeners on the platforms that
/// emit the events, of the family's `platforms`.
fn events(description: &Description, id: &str, family: Family, platforms: &[&str]) -> String {
    let event_type = |event: &Event| format!("{id}.{}.{}", family.name(), event.name);
    let mut source = "\ndeclare global {\n  // eslint-disable-next-line @typescript-eslint/no-namespace -- events are declared by merging into Tokamak.Events.\n  namespace Tokamak {\n    interface Events {\n".to_owned();
    for event in &description.events {
        let _ = writeln!(
            source,
            "      {:?}: {};",
            event_type(event),
            typescript_type(&event.ty, false)
        );
    }
    source.push_str("    }\n");
    let replies = description
        .events
        .iter()
        .filter_map(|event| Some((event, event.reply.as_ref()?)))
        .collect::<Vec<_>>();
    if !replies.is_empty() {
        source.push_str("    interface EventReplies {\n");
        for (event, reply) in replies {
            let _ = writeln!(
                source,
                "      {:?}: {};",
                event_type(event),
                typescript_type(reply, false)
            );
        }
        source.push_str("    }\n");
    }
    source.push_str("  }\n}\n");
    for event in &description.events {
        let emitting = if event.platforms.is_empty() {
            platforms.to_vec()
        } else {
            event.platforms.iter().map(String::as_str).collect()
        };
        let _ = write!(
            source,
            "\n{}export const {} = defineEvent({:?}, {{ platforms: {emitting:?} }});\n",
            documentation(&event.doc, &event.platforms, ""),
            event.listener(),
            event_type(event)
        );
    }
    source
}

fn interface_name(family: Family) -> &'static str {
    match family {
        Family::Apple => "Apple",
        Family::Android => "Android",
    }
}

/// The imports the generated source uses: one statement per module.
fn imports(description: &Description) -> String {
    let any = |test: &dyn Fn(&Method) -> bool| description.methods.iter().any(test);
    let fields = description
        .types
        .iter()
        .flat_map(|named| match &named.shape {
            Shape::Fields(fields) => fields.iter().map(|field| &field.ty).collect(),
            Shape::Values(_) => Vec::new(),
            Shape::Alternatives(alternatives) => alternatives
                .iter()
                .filter_map(|alternative| alternative.ty.as_ref())
                .collect(),
        });
    let mut types = description
        .methods
        .iter()
        .flat_map(|method| {
            method
                .parameters
                .iter()
                .map(|parameter| &parameter.ty)
                .chain(&method.returns)
        })
        .chain(
            description
                .events
                .iter()
                .flat_map(|event| std::iter::once(&event.ty).chain(&event.reply)),
        )
        .chain(fields);
    let references = |method: &Method| {
        method.kind == Kind::Resource
            || method
                .parameters
                .iter()
                .any(|parameter| parameter.ty == Type::Reference)
    };
    // Each name the generated source may import, with whether it does.
    let used = [
        ("NativeEndpoint", true),
        (
            "decodeJson",
            any(&|method| bytes_parameter(method).is_some() && method.returns != Some(Type::Bytes)),
        ),
        (
            "encodeJson",
            any(&|method| bytes_parameter(method).is_none() && method.returns == Some(Type::Bytes)),
        ),
        (
            "type JsonStream",
            any(&|method| method.kind == Kind::Stream),
        ),
        ("type JsonValue", types.any(mentions_json)),
        ("type NativeReference", any(&references)),
        ("type TokamakContext", true),
    ];
    let names = used
        .into_iter()
        .filter_map(|(name, used)| used.then_some(name))
        .collect::<Vec<_>>();
    let mut imports = format!(
        "import {{ {} }} from \"@tokamakdev/plugin\";\n",
        names.join(", ")
    );
    if !description.events.is_empty() {
        imports.push_str("import { defineEvent } from \"@tokamakdev/plugin/events\";\n");
    }
    imports
}

/// Whether `ty` is or holds any JSON value.
fn mentions_json(ty: &Type) -> bool {
    match ty {
        Type::Json => true,
        Type::Optional(inner) | Type::Array(inner) | Type::Record(inner) => mentions_json(inner),
        _ => false,
    }
}

/// The name of the method's parameter when it is its only one and bytes, which the call sends
/// as they are.
fn bytes_parameter(method: &Method) -> Option<&str> {
    match method.parameters.as_slice() {
        [
            Field {
                name,
                ty: Type::Bytes,
            },
        ] => Some(name),
        _ => None,
    }
}

fn type_declaration(named: &NamedType) -> String {
    match &named.shape {
        Shape::Values(values) => {
            let values = values
                .iter()
                .map(|value| format!("{value:?}"))
                .collect::<Vec<_>>()
                .join(" | ");
            format!("export type {} = {values};\n", named.name)
        }
        Shape::Fields(fields) => {
            let fields = fields.iter().map(field_declaration).collect::<String>();
            format!("export interface {} {{\n{fields}}}\n", named.name)
        }
        Shape::Alternatives(alternatives) => {
            let forms = alternatives
                .iter()
                .map(|alternative| form_type(named.tag.as_deref(), alternative))
                .collect::<Vec<_>>()
                .join(" | ");
            format!("export type {} = {forms};\n", named.name)
        }
    }
}

/// A union's form in TypeScript: its name, or its value, with the field `tag` naming it.
fn form_type(tag: Option<&str>, alternative: &Alternative) -> String {
    let name = &alternative.name;
    match (tag, &alternative.ty) {
        (None, None) => format!("{name:?}"),
        (None, Some(ty)) => typescript_type(ty, false),
        (Some(tag), None) => format!("{{ readonly {tag}: {name:?} }}"),
        (Some(tag), Some(ty)) => format!(
            "({{ readonly {tag}: {name:?} }} & {})",
            typescript_type(ty, false)
        ),
    }
}

/// A field of an interface, which a nil leaves out: native encoders omit it.
fn field_declaration(field: &Field) -> String {
    let (optional, ty) = match &field.ty {
        Type::Optional(wrapped) => ("?", wrapped.as_ref()),
        ty => ("", ty),
    };
    format!(
        "  readonly {}{optional}: {};\n",
        field.name,
        typescript_type(ty, false)
    )
}

/// `ty` in TypeScript. Arrays and records are readonly, except a method's result, which the
/// caller owns.
fn typescript_type(ty: &Type, owned: bool) -> String {
    let readonly = if owned { "" } else { "readonly " };
    match ty {
        Type::String => "string".into(),
        Type::Number => "number".into(),
        Type::Boolean => "boolean".into(),
        Type::Void => "void".into(),
        Type::Bytes => "Uint8Array".into(),
        Type::Body => "ReadableStream<Uint8Array>".into(),
        Type::Reference => "NativeReference".into(),
        Type::Json => "JsonValue".into(),
        Type::Optional(inner) => format!("{} | null", typescript_type(inner, owned)),
        Type::Array(element) if matches!(**element, Type::Optional(_)) => {
            format!("{readonly}({})[]", typescript_type(element, false))
        }
        Type::Array(element) => format!("{readonly}{}[]", typescript_type(element, false)),
        Type::Record(value) if owned => {
            format!("Record<string, {}>", typescript_type(value, false))
        }
        Type::Record(value) => format!(
            "Readonly<Record<string, {}>>",
            typescript_type(value, false)
        ),
        Type::Named(name) => name.clone(),
    }
}

/// The method's declaration in the interface, with its documentation.
fn signature(method: &Method) -> String {
    let parameters = method
        .parameters
        .iter()
        .enumerate()
        .map(|(index, parameter)| {
            let optional = if omittable(method, index) { "?" } else { "" };
            format!(
                "{}{optional}: {}",
                parameter.name,
                typescript_type(&parameter.ty, false)
            )
        })
        .collect::<Vec<_>>();
    let returns = match (method.kind, &method.returns) {
        (Kind::Call, Some(returns)) => format!("Promise<{}>", typescript_type(returns, true)),
        (Kind::Stream, Some(returns)) => format!("JsonStream<{}>", typescript_type(returns, false)),
        _ => "Promise<NativeReference>".into(),
    };
    format!(
        "{}  {}({}): {returns};\n",
        documentation(&method.doc, &method.platforms, "  "),
        method.name,
        parameters.join(", ")
    )
}

/// Whether the parameter at `index` may be left out: it and every parameter after it are optional.
fn omittable(method: &Method, index: usize) -> bool {
    method.parameters[index..]
        .iter()
        .all(|parameter| matches!(parameter.ty, Type::Optional(_)))
}

/// A method's or event's doc comment at `indent`, which names its platforms when the description
/// lists them: only when some of the family's platforms lack it.
fn documentation(doc: &str, platforms: &[String], indent: &str) -> String {
    let mut lines = doc
        .lines()
        .map(|line| line.replace("*/", "*\\/"))
        .collect::<Vec<_>>();
    if !platforms.is_empty() {
        lines.push(format!("@platforms {}", platforms.join(" ")));
    }
    match lines.as_slice() {
        [] => String::new(),
        [line] => format!("{indent}/** {line} */\n"),
        lines => {
            let mut comment = format!("{indent}/**\n");
            for line in lines {
                let space = if line.is_empty() { "" } else { " " };
                let _ = writeln!(comment, "{indent} *{space}{line}");
            }
            comment + indent + " */\n"
        }
    }
}

/// The arrow function that implements the method on the endpoint.
fn implementation(method: &Method) -> String {
    let parameters = method
        .parameters
        .iter()
        .map(|parameter| parameter.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let name = &method.name;
    match bytes_parameter(method) {
        Some(data) if method.returns == Some(Type::Bytes) => {
            format!("({parameters}) => endpoint.callBytes({name:?}, {data})")
        }
        Some(data) => format!(
            "async ({parameters}) => decodeJson(await endpoint.callBytes({name:?}, {data}))"
        ),
        None => format!("({parameters}) => {}", json_call(method)),
    }
}

/// A call, stream or resource whose arguments are a JSON object of the parameters, which may be
/// left out when there are none.
fn json_call(method: &Method) -> String {
    let name = &method.name;
    let fields = method
        .parameters
        .iter()
        .filter(|parameter| parameter.ty != Type::Body)
        .map(|parameter| match parameter.ty {
            Type::Reference => format!("{0}: endpoint.reference({0})", parameter.name),
            _ => parameter.name.clone(),
        })
        .collect::<Vec<_>>();
    let object = if fields.is_empty() {
        "{}".to_owned()
    } else {
        format!("{{ {} }}", fields.join(", "))
    };
    let body = match method.parameters.last() {
        Some(Field {
            ty: Type::Body,
            name,
        }) => format!(", {name}"),
        _ => String::new(),
    };
    let arguments = if fields.is_empty() && body.is_empty() {
        String::new()
    } else {
        format!(", {object}{body}")
    };
    match method.kind {
        Kind::Call if method.returns == Some(Type::Bytes) => {
            format!("endpoint.callBytes({name:?}, encodeJson({object}){body})")
        }
        Kind::Call => format!("endpoint.call({name:?}{arguments})"),
        Kind::Stream if method.overflow == Overflow::DropOldest => {
            format!("endpoint.stream({name:?}, {object}, \"dropOldest\")")
        }
        Kind::Stream => format!("endpoint.stream({name:?}{arguments})"),
        Kind::Resource => format!("endpoint.resource({name:?}{arguments})"),
    }
}

#[cfg(test)]
mod tests {
    use super::{Description, Family, typescript};
    use tokamak_cli::Platform;

    fn description(json: &str) -> anyhow::Result<Description> {
        Ok(serde_json::from_str(json)?)
    }

    #[test]
    fn generates_the_platform_api_fixture() -> anyhow::Result<()> {
        let description = description(include_str!(
            "../../plugins/core/tests/fixtures/description.json"
        ))?;
        description.validate()?;

        assert_eq!(
            typescript(
                &description,
                "example",
                Family::Apple,
                &[Platform::Ios, Platform::Macos]
            ),
            include_str!("../../plugins/core/tests/fixtures/apple.generated.ts")
        );
        Ok(())
    }

    #[test]
    fn sends_the_body_of_a_call_that_returns_bytes() -> anyhow::Result<()> {
        let description = description(
            r#"{"methods": [{"name": "upload", "kind": "call", "parameters": [{"name": "name", "type": "string"}, {"name": "file", "type": "body"}], "returns": "bytes"}]}"#,
        )?;
        description.validate()?;
        let source = typescript(
            &description,
            "example",
            Family::Apple,
            &[Platform::Ios, Platform::Macos],
        );
        assert!(
            source.contains(
                r#"upload: (name, file) => endpoint.callBytes("upload", encodeJson({ name }), file),"#
            ),
            "{source}"
        );
        Ok(())
    }

    #[test]
    fn rejects_methods_the_platform_api_cannot_express() -> anyhow::Result<()> {
        let cases = [
            (
                r#"{"methods": [{"name": "then", "kind": "call", "returns": "void"}]}"#,
                "promise",
            ),
            (
                r#"{"methods": [{"name": "a", "kind": "call", "parameters": [{"name": "endpoint", "type": "string"}], "returns": "void"}]}"#,
                "reserved name",
            ),
            (
                r#"{"methods": [{"name": "a", "kind": "call", "parameters": [{"name": "encodeJson", "type": "string"}], "returns": "void"}]}"#,
                "reserved name",
            ),
            (
                r#"{"methods": [{"name": "a", "kind": "call", "parameters": [{"name": "arguments", "type": "string"}], "returns": "void"}]}"#,
                "reserved name",
            ),
            (
                r#"{"methods": [], "types": [{"name": "Apple", "values": ["a"]}]}"#,
                "the generated API declares",
            ),
            (
                r#"{"methods": [{"name": "a", "kind": "call", "parameters": [{"name": "data", "type": "bytes"}, {"name": "b", "type": "string"}], "returns": "void"}]}"#,
                "only parameter",
            ),
            (
                r#"{"methods": [{"name": "a", "kind": "call", "parameters": [{"name": "file", "type": "body"}, {"name": "b", "type": "string"}], "returns": "void"}]}"#,
                "last parameter",
            ),
            (
                r#"{"methods": [{"name": "a", "kind": "stream", "returns": "void"}]}"#,
                "only a call's result",
            ),
            (
                r#"{"methods": [{"name": "a", "kind": "call", "returns": {"array": "bytes"}}]}"#,
                "only parameter or its result",
            ),
            (
                r#"{"methods": [{"name": "a", "kind": "call", "returns": {"named": "Missing"}}]}"#,
                "not described",
            ),
            (
                r#"{"methods": [{"name": "a", "kind": "call", "returns": {"optional": {"optional": "string"}}}]}"#,
                "optional of an optional",
            ),
            (
                r#"{"methods": [{"name": "a", "kind": "call", "returns": "void"}, {"name": "a", "kind": "call", "returns": "void"}]}"#,
                "exported twice",
            ),
            (
                r#"{"methods": [], "types": [{"name": "Kind", "values": []}]}"#,
                "no cases",
            ),
            (
                r#"{"methods": [], "types": [{"name": "Kind", "values": ["high", "high"]}]}"#,
                "two cases named high",
            ),
        ];
        assert_rejected(&cases)
    }

    #[test]
    fn rejects_events_the_platform_api_cannot_express() -> anyhow::Result<()> {
        assert_rejected(&[
            (
                r#"{"methods": [], "types": [{"name": "defineEvent", "values": ["a"]}]}"#,
                "the generated API declares",
            ),
            (
                r#"{"methods": [], "events": [{"name": "on-update", "type": "string"}]}"#,
                "not a JavaScript identifier",
            ),
            (
                r#"{"methods": [], "events": [{"name": "update", "type": "string"}, {"name": "Update", "type": "number"}]}"#,
                "two events register onUpdate",
            ),
            (
                r#"{"methods": [], "events": [{"name": "update", "type": "void"}]}"#,
                "only a call's result",
            ),
            (
                r#"{"methods": [], "events": [{"name": "update", "type": "string", "reply": "bytes"}]}"#,
                "event update: reply: bytes",
            ),
            (
                r#"{"methods": [], "events": [{"name": "update", "type": "reference"}]}"#,
                "event update: a native reference is a parameter",
            ),
            (
                r#"{"methods": [], "events": [{"name": "update", "type": {"named": "Missing"}}]}"#,
                "event update: type Missing is not described",
            ),
        ])
    }

    #[test]
    fn declares_unions_by_their_forms() -> anyhow::Result<()> {
        let description = description(
            r#"{"methods": [{"name": "show", "kind": "call", "parameters": [{"name": "sound", "type": {"named": "Sound"}}, {"name": "style", "type": {"named": "Style"}}], "returns": "void"}], "types": [
                {"name": "Sound", "alternatives": [{"name": "default"}, {"name": "named", "type": {"named": "NamedSound"}}, {"name": "volume", "type": "number"}]},
                {"name": "NamedSound", "fields": [{"name": "name", "type": "string"}]},
                {"name": "Style", "tag": "type", "alternatives": [{"name": "plain"}, {"name": "bigText", "type": {"named": "BigText"}}]},
                {"name": "BigText", "fields": [{"name": "text", "type": "string"}]}
            ]}"#,
        )?;
        description.validate()?;
        let source = typescript(
            &description,
            "example",
            Family::Android,
            &[Platform::Android],
        );
        assert!(
            source.contains("export type Sound = \"default\" | NamedSound | number;\n"),
            "{source}"
        );
        assert!(
            source.contains(
                "export type Style = { readonly type: \"plain\" } | ({ readonly type: \"bigText\" } & BigText);\n"
            ),
            "{source}"
        );
        Ok(())
    }

    #[test]
    fn rejects_unions_whose_forms_are_indistinguishable() -> anyhow::Result<()> {
        let union = |forms: &str, others: &str| {
            format!(
                r#"{{"methods": [], "types": [{{"name": "U", "alternatives": [{forms}]}}, {{"name": "A", "fields": [{{"name": "a", "type": "string"}}, {{"name": "b", "type": {{"optional": "string"}}}}]}}, {{"name": "B", "fields": [{{"name": "b", "type": "string"}}]}}, {{"name": "Kind", "values": ["x"]}}{others}]}}"#
            )
        };
        let cases = [
            (union("", ""), "no forms"),
            (
                union(r#"{"name": "x", "type": "boolean"}, {"name": "y", "type": "boolean"}"#, ""),
                "the same JSON",
            ),
            (
                union(r#"{"name": "x"}, {"name": "kind", "type": {"named": "Kind"}}"#, ""),
                "the same JSON",
            ),
            (
                union(r#"{"name": "x"}, {"name": "y", "type": "string"}"#, ""),
                "string form takes",
            ),
            (
                union(
                    r#"{"name": "x", "type": {"record": "number"}}, {"name": "a", "type": {"named": "A"}}"#,
                    "",
                ),
                "object form takes",
            ),
            (
                union(
                    r#"{"name": "a", "type": {"named": "A"}}, {"name": "b", "type": {"named": "B"}}"#,
                    "",
                ),
                "B has no required field",
            ),
            (
                union(
                    r#"{"name": "a", "type": {"named": "A"}}, {"name": "c", "type": {"named": "A"}}"#,
                    "",
                ),
                "A has no required field",
            ),
            (
                union(r#"{"name": "x", "type": {"optional": "number"}}"#, ""),
                "U.x: a union's form is not optional",
            ),
            (
                union(r#"{"name": "x", "type": "json"}"#, ""),
                "not any JSON value",
            ),
            (
                union(
                    r#"{"name": "x", "type": {"named": "V"}}"#,
                    r#", {"name": "V", "alternatives": [{"name": "y"}]}"#,
                ),
                "not a union",
            ),
            (
                union(r#"{"name": "x"}, {"name": "x", "type": "number"}"#, ""),
                "two forms named x",
            ),
            (
                r#"{"methods": [], "types": [{"name": "A", "tag": "type", "fields": []}]}"#.to_owned(),
                "has a tag but is not a union",
            ),
            (
                r#"{"methods": [], "types": [{"name": "U", "tag": "type", "alternatives": [{"name": "x", "type": "boolean"}]}]}"#.to_owned(),
                "is a name or a struct",
            ),
            (
                r#"{"methods": [], "types": [{"name": "U", "tag": "a", "alternatives": [{"name": "x", "type": {"named": "A"}}]}, {"name": "A", "fields": [{"name": "a", "type": "string"}]}]}"#.to_owned(),
                "A has the field a, which tags U's forms",
            ),
        ];
        assert_rejected(&cases)
    }

    /// Checks that validation rejects each description with an error containing its message.
    fn assert_rejected(cases: &[(impl AsRef<str>, &str)]) -> anyhow::Result<()> {
        for (json, message) in cases {
            let json = json.as_ref();
            let error = description(json)?
                .validate()
                .err()
                .ok_or_else(|| anyhow::anyhow!("accepted {json}"))?;
            assert!(format!("{error:#}").contains(message), "{error:#}");
        }
        Ok(())
    }
}
