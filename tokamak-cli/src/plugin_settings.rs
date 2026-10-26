//! Plugin build settings: the kinds of value they take, and the parsing and
//! checks of their values.
//!
//! Options and environment variables give text, which one parser per kind
//! reads; the `tokamak()` options give JSON, which is checked against the
//! kind. A list's and a map's items are comma-separated in text, and a map's
//! are `key=value`.

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{Result, anyhow};
use clap::builder::{PossibleValuesParser, StringValueParser, TypedValueParser, ValueParser};
use clap::{Arg, ArgAction, ArgMatches};
use serde::Deserialize;
use serde_json::{Number, Value};
use tokamak_cli::Platform;

use super::tokamak_config::{VALUE_FORM, value_problem};

/// A plugin's build setting, as its manifest declares it.
#[derive(Clone, Debug, Deserialize)]
#[serde(try_from = "RawSetting")]
pub(crate) struct Setting {
    pub(crate) kind: Kind,
    /// One-line description shown by `--help`.
    pub(crate) description: String,
    /// The value when none is set, of the setting's kind.
    pub(crate) default: Option<Value>,
}

/// The values a setting takes: one item, or a list or map of items.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Kind {
    One(Item),
    List(Item),
    Map(Item),
}

/// The kind of a single value, or of a list's or map's items.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Item {
    Boolean,
    Integer,
    Number,
    String,
    /// A file or directory, made absolute when the setting resolves.
    Path,
    /// One of the values.
    Enum(Vec<String>),
}

/// A setting as the manifest writes it.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSetting {
    kind: String,
    /// A list's or map's item kind.
    of: Option<String>,
    /// An enum's values.
    values: Option<Vec<String>>,
    description: String,
    default: Option<Value>,
}

impl TryFrom<RawSetting> for Setting {
    type Error = String;

    fn try_from(raw: RawSetting) -> Result<Self, String> {
        let (of_kind, item): (fn(Item) -> Kind, &str) = match (raw.kind.as_str(), &raw.of) {
            ("list", Some(of)) => (Kind::List, of),
            ("map", Some(of)) => (Kind::Map, of),
            ("list" | "map", None) => {
                return Err(format!("a {} needs `of`, its items' kind", raw.kind));
            }
            (_, Some(_)) => return Err("only a list or map takes `of`".to_owned()),
            (kind, None) => (Kind::One, kind),
        };
        let kind = of_kind(Item::declared(item, raw.values)?);
        if raw.description.trim().is_empty() {
            return Err("a setting needs a description".to_owned());
        }
        if let Some(default) = &raw.default {
            kind.check(default)
                .map_err(|problem| format!("default {problem}"))?;
        }
        Ok(Self {
            kind,
            description: raw.description,
            default: raw.default,
        })
    }
}

impl Item {
    /// The item kind named `name`, with an enum's `values`.
    fn declared(name: &str, values: Option<Vec<String>>) -> Result<Self, String> {
        match (name, values) {
            ("enum", Some(values))
                if !values.is_empty()
                    && values.iter().all(|value| value_problem(value).is_none()) =>
            {
                Ok(Self::Enum(values))
            }
            ("enum", _) => Err(format!("an enum needs `values`, each {VALUE_FORM}")),
            (_, Some(_)) => Err("only an enum takes `values`".to_owned()),
            ("boolean", None) => Ok(Self::Boolean),
            ("integer", None) => Ok(Self::Integer),
            ("number", None) => Ok(Self::Number),
            ("string", None) => Ok(Self::String),
            ("path", None) => Ok(Self::Path),
            (name, None) => Err(format!(
                "unknown kind `{name}`; a setting is a boolean, integer, number, string, path, \
                 enum, or a list or map of one of those"
            )),
        }
    }

    /// The value `text` sets, or what it must be.
    fn parse(&self, text: &str) -> Result<Value, String> {
        let value = match self {
            Self::Boolean => text.parse::<bool>().ok().map(Value::Bool),
            Self::Integer => text.parse::<i64>().ok().map(Value::from),
            Self::Number => text
                .parse::<f64>()
                .ok()
                .and_then(Number::from_f64)
                .map(Value::Number),
            Self::String | Self::Path => value_problem(text).is_none().then(|| text.into()),
            Self::Enum(values) => values
                .iter()
                .any(|value| value == text)
                .then(|| text.into()),
        };
        value.ok_or_else(|| format!("must be {}", self.expected()))
    }

    /// Whether the configured `value` is of this kind.
    fn accepts(&self, value: &Value) -> bool {
        match (self, value) {
            (Self::Boolean, Value::Bool(_)) | (Self::Number, Value::Number(_)) => true,
            (Self::Integer, Value::Number(number)) => number.is_i64(),
            (Self::String | Self::Path | Self::Enum(_), Value::String(text)) => {
                self.parse(text).is_ok()
            }
            _ => false,
        }
    }

    fn expected(&self) -> String {
        match self {
            Self::Boolean => "true or false".to_owned(),
            Self::Integer => "a whole number".to_owned(),
            Self::Number => "a finite number".to_owned(),
            Self::String | Self::Path => VALUE_FORM.to_owned(),
            Self::Enum(values) => format!("one of {}", values.join(", ")),
        }
    }

    /// The parser of an option's value, which lists a boolean's or enum's
    /// values in help and suggests one for a mistyped value.
    fn value_parser(&self) -> ValueParser {
        let item = self.clone();
        let parse = move |text: String| item.parse(&text);
        match self {
            Self::Boolean => PossibleValuesParser::new(["true", "false"])
                .try_map(parse)
                .into(),
            Self::Enum(values) => PossibleValuesParser::new(values.clone())
                .try_map(parse)
                .into(),
            _ => StringValueParser::new().try_map(parse).into(),
        }
    }

    fn value_name(&self) -> &'static str {
        match self {
            Self::Boolean => "BOOLEAN",
            Self::Integer => "INTEGER",
            Self::Number => "NUMBER",
            Self::Path => "PATH",
            Self::String | Self::Enum(_) => "VALUE",
        }
    }
}

impl Kind {
    fn item(&self) -> &Item {
        match self {
            Self::One(item) | Self::List(item) | Self::Map(item) => item,
        }
    }

    /// The value an option's or environment variable's `text` sets, or what
    /// it must be.
    pub(crate) fn parse(&self, text: &str) -> Result<Value, String> {
        let parsed = match self {
            Self::One(item) => item.parse(text).ok(),
            Self::List(item) => text
                .split(',')
                .map(|text| item.parse(text).ok())
                .collect::<Option<_>>()
                .map(Value::Array),
            Self::Map(item) => text
                .split(',')
                .map(|text| pair(item, text).ok())
                .collect::<Option<_>>()
                .map(Value::Object),
        };
        parsed.ok_or_else(|| {
            let item = self.item().expected();
            match self {
                Self::One(_) => format!("must be {item}"),
                Self::List(_) => format!("must be comma-separated values, each {item}"),
                Self::Map(_) => {
                    format!("must be comma-separated key=value pairs, each value {item}")
                }
            }
        })
    }

    /// Checks a value from the `tokamak()` options, returning what it must be.
    pub(crate) fn check(&self, value: &Value) -> Result<(), String> {
        let accepted = match (self, value) {
            (Self::One(item), value) => item.accepts(value),
            (Self::List(item), Value::Array(items)) => {
                items.iter().all(|value| item.accepts(value))
            }
            (Self::Map(item), Value::Object(entries)) => entries
                .iter()
                .all(|(key, value)| value_problem(key).is_none() && item.accepts(value)),
            _ => false,
        };
        if accepted {
            return Ok(());
        }
        let item = self.item().expected();
        Err(match self {
            Self::One(_) => format!("must be {item}"),
            Self::List(_) => format!("must be an array of values, each {item}"),
            Self::Map(_) => format!("must be an object of values, each {item}"),
        })
    }

    /// `value`, of this kind, with each path made absolute against `directory`.
    pub(crate) fn resolve_paths(&self, value: Value, directory: &Path) -> Result<Value> {
        if *self.item() != Item::Path {
            return Ok(value);
        }
        let absolute = |value: Value| match value {
            Value::String(path) => directory
                .join(path)
                .into_os_string()
                .into_string()
                .map(Value::String)
                .map_err(|_| anyhow!("must be a UTF-8 path")),
            value => Ok(value),
        };
        Ok(match value {
            Value::Array(items) => {
                Value::Array(items.into_iter().map(absolute).collect::<Result<_>>()?)
            }
            Value::Object(entries) => Value::Object(
                entries
                    .into_iter()
                    .map(|(key, value)| Ok((key, absolute(value)?)))
                    .collect::<Result<_>>()?,
            ),
            value => absolute(value)?,
        })
    }

    /// The value `matches` holds for the option `name`, which
    /// [`Kind::argument`] made.
    pub(crate) fn option_value(&self, matches: &ArgMatches, name: &str) -> Value {
        match self {
            Self::One(_) => matches.get_one::<Value>(name).cloned().unwrap_or_default(),
            Self::List(_) => Value::Array(
                matches
                    .get_many::<Value>(name)
                    .into_iter()
                    .flatten()
                    .cloned()
                    .collect(),
            ),
            Self::Map(_) => Value::Object(
                matches
                    .get_many::<(String, Value)>(name)
                    .into_iter()
                    .flatten()
                    .cloned()
                    .collect(),
            ),
        }
    }

    /// The option `--<name>`, which sets a value of this kind: a boolean is a
    /// flag that also takes `=true` or `=false`, and a list's or map's items
    /// repeat or are comma-separated.
    pub(crate) fn argument(&self, name: String) -> Arg {
        let item = self.item().clone();
        let argument = Arg::new(name.clone())
            .long(name)
            .allow_negative_numbers(matches!(item, Item::Integer | Item::Number))
            .value_parser(item.value_parser())
            .value_name(item.value_name());
        match self {
            Self::One(Item::Boolean) => argument
                .num_args(0..=1)
                .require_equals(true)
                .default_missing_value("true"),
            Self::One(_) => argument,
            Self::List(_) => argument.action(ArgAction::Append).value_delimiter(','),
            Self::Map(_) => argument
                .action(ArgAction::Append)
                .value_delimiter(',')
                .value_parser(StringValueParser::new().try_map(move |text| pair(&item, &text)))
                .value_name("KEY=VALUE"),
        }
    }
}

/// The key and value of a map's `key=value` item.
fn pair(item: &Item, text: &str) -> Result<(String, Value), String> {
    let (key, value) = text
        .split_once('=')
        .ok_or_else(|| "must be key=value".to_owned())?;
    if let Some(problem) = value_problem(key) {
        return Err(format!("key {problem}"));
    }
    Ok((key.to_owned(), item.parse(value)?))
}

/// `value`, of a setting's kind, as an option writes it.
pub(crate) fn option_text(value: &Value) -> String {
    let text = |value: &Value| match value {
        Value::String(text) => text.clone(),
        value => value.to_string(),
    };
    match value {
        Value::Array(items) => items.iter().map(text).collect::<Vec<_>>().join(","),
        Value::Object(entries) => entries
            .iter()
            .map(|(key, value)| format!("{key}={}", text(value)))
            .collect::<Vec<_>>()
            .join(","),
        value => text(value),
    }
}

/// The namespaces of a plugin setting's options: none, for every platform,
/// then each platform's, once.
pub(crate) fn option_namespaces() -> impl Iterator<Item = Option<&'static str>> {
    let platforms = Platform::ALL
        .iter()
        .map(|platform| platform.namespace())
        .collect::<BTreeSet<_>>();
    std::iter::once(None).chain(platforms.into_iter().map(Some))
}

/// The name, without `--`, of the option that sets plugin `id`'s `key` for
/// every platform, or for the platform `namespace`.
pub(crate) fn option_name(id: &str, namespace: Option<&str>, key: &str) -> String {
    match namespace {
        Some(namespace) => format!("{id}-{namespace}-{key}"),
        None => format!("{id}-{key}"),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use anyhow::Result;
    use serde_json::{Value, json};

    use super::{Item, Kind, Setting};

    fn setting(manifest: &Value) -> Result<Setting, String> {
        serde_json::from_value(manifest.clone()).map_err(|error| error.to_string())
    }

    fn kind(manifest: &Value) -> Result<Kind> {
        Ok(setting(manifest).map_err(anyhow::Error::msg)?.kind)
    }

    #[test]
    fn reads_each_kind_from_the_manifest() -> Result<()> {
        let enumerated = || Item::Enum(vec!["precise".to_owned(), "approximate".to_owned()]);
        for (manifest, expected) in [
            (json!({ "kind": "boolean" }), Kind::One(Item::Boolean)),
            (json!({ "kind": "integer" }), Kind::One(Item::Integer)),
            (json!({ "kind": "number" }), Kind::One(Item::Number)),
            (json!({ "kind": "string" }), Kind::One(Item::String)),
            (json!({ "kind": "path" }), Kind::One(Item::Path)),
            (
                json!({ "kind": "enum", "values": ["precise", "approximate"] }),
                Kind::One(enumerated()),
            ),
            (
                json!({ "kind": "list", "of": "path" }),
                Kind::List(Item::Path),
            ),
            (
                json!({ "kind": "map", "of": "enum", "values": ["precise", "approximate"] }),
                Kind::Map(enumerated()),
            ),
        ] {
            let mut manifest = manifest;
            manifest["description"] = "A setting".into();
            assert_eq!(kind(&manifest)?, expected, "{manifest}");
        }
        Ok(())
    }

    #[test]
    fn rejects_malformed_settings_and_defaults_of_another_kind() {
        for (manifest, message) in [
            (json!({ "kind": "list" }), "a list needs `of`"),
            (
                json!({ "kind": "map", "of": "list" }),
                "unknown kind `list`",
            ),
            (
                json!({ "kind": "string", "of": "string" }),
                "only a list or map takes `of`",
            ),
            (json!({ "kind": "enum" }), "an enum needs `values`"),
            (
                json!({ "kind": "enum", "values": [] }),
                "an enum needs `values`",
            ),
            (
                json!({ "kind": "enum", "values": [" a"] }),
                "an enum needs `values`",
            ),
            (
                json!({ "kind": "string", "values": ["a"] }),
                "only an enum takes `values`",
            ),
            (json!({ "kind": "float" }), "unknown kind `float`"),
            (
                json!({ "kind": "boolean", "default": "true" }),
                "default must be true or false",
            ),
            (
                json!({ "kind": "list", "of": "integer", "default": [1.5] }),
                "default must be an array of values, each a whole number",
            ),
            (
                json!({ "kind": "string", "secret": true }),
                "unknown field `secret`",
            ),
        ] {
            let mut manifest = manifest;
            manifest["description"] = "A setting".into();
            let error = setting(&manifest).err().unwrap_or_default();
            assert!(error.contains(message), "{manifest}: {error}");
        }
        let error = setting(&json!({ "kind": "string", "description": " " }))
            .err()
            .unwrap_or_default();
        assert!(error.contains("a setting needs a description"), "{error}");
    }

    #[test]
    fn parses_text_into_each_kind() {
        let precision = Kind::One(Item::Enum(vec![
            "precise".to_owned(),
            "approximate".to_owned(),
        ]));
        for (kind, text, expected) in [
            (Kind::One(Item::Boolean), "false", json!(false)),
            (Kind::One(Item::Integer), "-12", json!(-12)),
            (Kind::One(Item::Number), "2.5", json!(2.5)),
            (Kind::One(Item::String), "a=b", json!("a=b")),
            (precision.clone(), "approximate", json!("approximate")),
            (Kind::List(Item::Integer), "1,2,3", json!([1, 2, 3])),
            (
                Kind::Map(Item::String),
                "Run=Tracks runs,Map=Shows a=b",
                json!({ "Run": "Tracks runs", "Map": "Shows a=b" }),
            ),
        ] {
            assert_eq!(kind.parse(text), Ok(expected), "{kind:?} {text}");
        }
        for (kind, text, message) in [
            (Kind::One(Item::Boolean), "yes", "must be true or false"),
            (Kind::One(Item::Integer), "1.5", "must be a whole number"),
            (Kind::One(Item::Number), "inf", "must be a finite number"),
            (Kind::One(Item::String), " x", "must be a non-empty value"),
            (precision, "precsie", "must be one of precise, approximate"),
            (
                Kind::List(Item::Integer),
                "1,,2",
                "must be comma-separated values, each a whole number",
            ),
            (
                Kind::Map(Item::String),
                "Run",
                "must be comma-separated key=value pairs, each value",
            ),
            (
                Kind::Map(Item::String),
                "=Tracks",
                "must be comma-separated key=value pairs",
            ),
        ] {
            let problem = kind.parse(text).err().unwrap_or_default();
            assert!(problem.starts_with(message), "{kind:?} {text}: {problem}");
        }
    }

    #[test]
    fn checks_configured_values_against_each_kind() {
        for (kind, value) in [
            (Kind::One(Item::Boolean), json!(true)),
            (Kind::One(Item::Integer), json!(3)),
            (Kind::One(Item::Number), json!(3)),
            (Kind::List(Item::String), json!([])),
            (Kind::Map(Item::String), json!({ "Run": "Tracks runs" })),
        ] {
            assert_eq!(kind.check(&value), Ok(()), "{kind:?} {value}");
        }
        for (kind, value, message) in [
            (
                Kind::One(Item::Boolean),
                json!("true"),
                "must be true or false",
            ),
            (
                Kind::One(Item::Integer),
                json!(3.5),
                "must be a whole number",
            ),
            (
                Kind::One(Item::String),
                json!(3),
                "must be a non-empty value",
            ),
            (
                Kind::List(Item::String),
                json!("a"),
                "must be an array of values",
            ),
            (
                Kind::List(Item::String),
                json!(["a", ""]),
                "must be an array of values",
            ),
            (
                Kind::Map(Item::String),
                json!(["a"]),
                "must be an object of values",
            ),
            (
                Kind::Map(Item::String),
                json!({ " ": "a" }),
                "must be an object of values",
            ),
        ] {
            let problem = kind.check(&value).err().unwrap_or_default();
            assert!(problem.starts_with(message), "{kind:?} {value}: {problem}");
        }
    }

    #[test]
    fn makes_paths_absolute() -> Result<()> {
        let directory = Path::new("/root");
        let path = |relative: &str| directory.join(relative).to_string_lossy().into_owned();
        for (kind, value, expected) in [
            (Kind::One(Item::Path), json!("a"), json!(path("a"))),
            (
                Kind::List(Item::Path),
                json!(["a", "/b"]),
                json!([path("a"), "/b"]),
            ),
            (
                Kind::Map(Item::Path),
                json!({ "x": "a" }),
                json!({ "x": path("a") }),
            ),
            (Kind::One(Item::String), json!("a"), json!("a")),
        ] {
            assert_eq!(kind.resolve_paths(value, directory)?, expected, "{kind:?}");
        }
        Ok(())
    }
}
