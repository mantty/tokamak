#![allow(clippy::needless_pass_by_value)]

use crate::typed::{JsFunction, StringPairs};
use rquickjs::prelude::List;
use rquickjs::{Ctx, Exception, FromJs, Function, IntoJs, Persistent, qjs};
use std::cell::RefCell;
use std::collections::HashMap;
use std::ptr::NonNull;
use std::rc::Rc;
use tokamak_macros::{JsType, host_function};
use url::Url;
use urlpattern::parser::RegexSyntax;
use urlpattern::quirks::{self, StringOrInit};
use urlpattern::regexp::RegExp;
use urlpattern::{UrlPatternInit, UrlPatternOptions};

/// A compiled pattern, whose regular expressions are the engine's.
type Pattern = urlpattern::UrlPattern<EngineRegExp>;

const PATTERN_CACHE_LIMIT: usize = 512;

/// Runs an ECMAScript regular expression's source with its flags on a text:
/// the UTF-16 start and end of the match and each capture, or `undefined`
/// without a match.
type Runner<'js> =
    JsFunction<'js, (&'js str, &'js str, &'js str), Option<Vec<Option<List<(usize, usize)>>>>>;

/// The context compiling a pattern and its JavaScript regular expression
/// runner; see [`EngineRegExp`].
#[derive(Clone)]
struct Compiling {
    context: NonNull<qjs::JSContext>,
    runner: Persistent<Function<'static>>,
}

thread_local! {
    static PATTERNS: RefCell<HashMap<String, Rc<Pattern>>> = RefCell::new(HashMap::new());
    static COMPILING: RefCell<Option<Compiling>> = const { RefCell::new(None) };
}

/// Restores the compiling context it replaced when dropped.
struct RestoreCompiling(Option<Compiling>);

impl Drop for RestoreCompiling {
    fn drop(&mut self) {
        COMPILING.set(self.0.take());
    }
}

tokamak_macros::host_functions! {
    pub(super),
    "urlParse" => url_parse,
    "urlSetComponent" => url_set_component,
    "urlEncodeParams" => encode_params,
    "urlDecodeParams" => decode_params,
    "urlPatternCompile" => pattern_compile,
    "urlPatternMatchInput" => pattern_match_input,
}

fn invalid_url(ctx: &Ctx<'_>, input: &str) -> rquickjs::Error {
    Exception::throw_type(ctx, &format!("Invalid URL: {input}"))
}

/// WHATWG URL accessor values.
#[derive(IntoJs, JsType)]
struct UrlParts {
    href: String,
    protocol: String,
    username: String,
    password: String,
    host: String,
    hostname: String,
    port: String,
    pathname: String,
    search: String,
    hash: String,
    origin: String,
}

impl UrlParts {
    /// `url`'s accessor values, via the url crate's spec-shaped quirks module.
    fn new(url: &Url) -> Self {
        use url::quirks;
        Self {
            href: quirks::href(url).to_owned(),
            protocol: quirks::protocol(url).to_owned(),
            username: quirks::username(url).to_owned(),
            password: quirks::password(url).to_owned(),
            host: quirks::host(url).to_owned(),
            hostname: quirks::hostname(url).to_owned(),
            port: quirks::port(url).to_owned(),
            pathname: quirks::pathname(url).to_owned(),
            search: quirks::search(url).to_owned(),
            hash: quirks::hash(url).to_owned(),
            origin: quirks::origin(url),
        }
    }
}

fn parse_url(ctx: &Ctx<'_>, input: &str, base: Option<&str>) -> rquickjs::Result<Url> {
    let base = base
        .map(|value| Url::parse(value).map_err(|_| invalid_url(ctx, value)))
        .transpose()?;
    Url::options()
        .base_url(base.as_ref())
        .parse(input)
        .map_err(|_| invalid_url(ctx, input))
}

#[host_function]
fn url_parse(ctx: Ctx<'_>, input: String, base: Option<String>) -> rquickjs::Result<UrlParts> {
    let url = parse_url(&ctx, &input, base.as_deref())?;
    Ok(UrlParts::new(&url))
}

/// Applies a WHATWG component-set algorithm and returns the resulting
/// components. Per the spec, setter failures leave the URL unchanged; only
/// the href setter throws on invalid input.
#[host_function]
fn url_set_component(
    ctx: Ctx<'_>,
    href: String,
    component: String,
    value: String,
) -> rquickjs::Result<UrlParts> {
    use url::quirks;
    let mut url = Url::parse(&href).map_err(|_| invalid_url(&ctx, &href))?;
    match component.as_str() {
        "href" => quirks::set_href(&mut url, &value).map_err(|_| invalid_url(&ctx, &value))?,
        "protocol" => drop(quirks::set_protocol(&mut url, &value)),
        "username" => drop(quirks::set_username(&mut url, &value)),
        "password" => drop(quirks::set_password(&mut url, &value)),
        "host" => drop(quirks::set_host(&mut url, &value)),
        "hostname" => drop(quirks::set_hostname(&mut url, &value)),
        "port" => drop(quirks::set_port(&mut url, &value)),
        "pathname" => quirks::set_pathname(&mut url, &value),
        "search" => quirks::set_search(&mut url, &value),
        "hash" => quirks::set_hash(&mut url, &value),
        _ => return Err(Exception::throw_type(&ctx, "Unknown URL component")),
    }
    Ok(UrlParts::new(&url))
}

/// Serializes name and value pairs as application/x-www-form-urlencoded.
#[host_function]
fn encode_params(entries: StringPairs) -> String {
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    serializer.extend_pairs(entries.0);
    serializer.finish()
}

/// Parses application/x-www-form-urlencoded input into name and value pairs.
#[host_function]
fn decode_params(input: String) -> StringPairs {
    StringPairs(
        url::form_urlencoded::parse(input.as_bytes())
            .into_owned()
            .collect(),
    )
}

fn pattern_key(input: &str, base: Option<&str>, ignore_case: bool) -> String {
    format!("{ignore_case}\u{1}{}\u{1}{input}", base.unwrap_or("\u{2}"))
}

/// An ECMAScript regular expression, which the engine compiles and runs.
/// The crate runs one while it parses a pattern; matching happens in
/// JavaScript.
struct EngineRegExp {
    source: String,
    flags: String,
}

impl RegExp for EngineRegExp {
    fn syntax() -> RegexSyntax {
        RegexSyntax::EcmaScript
    }

    fn parse(source: &str, flags: &str, force_eval: bool) -> Result<Self, ()> {
        if force_eval {
            run(source, flags, "")?;
        }
        Ok(Self {
            source: source.to_owned(),
            flags: flags.to_owned(),
        })
    }

    fn matches<'a>(&self, text: &'a str) -> Option<Vec<Option<&'a str>>> {
        let bounds = run(&self.source, &self.flags, text).ok()??;
        let slice =
            |List((start, end))| Some(&text[byte_offset(text, start)?..byte_offset(text, end)?]);
        // The first bounds are the whole match's.
        Some(
            bounds
                .into_iter()
                .skip(1)
                .map(|bounds| bounds.and_then(slice))
                .collect(),
        )
    }

    fn pattern_string(&self) -> &str {
        &self.source
    }
}

/// Run `source` with `flags` on `text` through the compiling call's runner:
/// the UTF-16 start and end of the match and each capture, or `None` without
/// a match.
#[allow(clippy::type_complexity)]
fn run(
    source: &str,
    flags: &str,
    text: &str,
) -> Result<Option<Vec<Option<List<(usize, usize)>>>>, ()> {
    let Compiling { context, runner } = COMPILING.with_borrow(Clone::clone).ok_or(())?;
    // SAFETY: `compile_pattern` names its context here only while it runs.
    let ctx = unsafe { Ctx::from_raw(context) };
    let runner = runner.restore(&ctx).map_err(|_| ())?;
    let runner = Runner::from_js(&ctx, runner.into_value()).map_err(|_| ())?;
    runner.call((source, flags, text)).map_err(|_| {
        // An invalid expression throws; the crate reports it instead.
        ctx.catch();
    })
}

/// The byte offset in `text` of UTF-16 offset `index`.
fn byte_offset(text: &str, index: usize) -> Option<usize> {
    let mut units = 0;
    for (offset, character) in text.char_indices() {
        if units == index {
            return Some(offset);
        }
        units += character.len_utf16();
    }
    (units == index).then_some(text.len())
}

fn compile_pattern<'js>(
    ctx: &Ctx<'js>,
    input: &str,
    base: Option<&str>,
    ignore_case: bool,
    runner: Runner<'js>,
) -> rquickjs::Result<Pattern> {
    let input: StringOrInit = serde_json::from_str(input).map_err(|error| {
        Exception::throw_type(ctx, &format!("Invalid URLPattern input: {error}"))
    })?;
    let options = UrlPatternOptions {
        ignore_case,
        ..UrlPatternOptions::default()
    };
    let compiling = Compiling {
        context: ctx.as_raw(),
        runner: Persistent::save(ctx, runner.into_inner()),
    };
    let _restore = RestoreCompiling(COMPILING.replace(Some(compiling)));
    construct_input(input, base)
        .and_then(|init| Pattern::parse(init, options))
        .map_err(|error| Exception::throw_type(ctx, &error.to_string()))
}

/// The pattern `input` describes, as `quirks::process_construct_pattern_input`
/// builds it, but parsing a constructor string with the engine's regular
/// expressions; the quirks function parses one with the Rust regex crate.
fn construct_input(
    input: StringOrInit<'_>,
    base: Option<&str>,
) -> Result<UrlPatternInit, urlpattern::Error> {
    let init = match input {
        StringOrInit::String(pattern) => {
            let base = base
                .map(Url::parse)
                .transpose()
                .map_err(urlpattern::Error::Url)?;
            return UrlPatternInit::parse_constructor_string::<EngineRegExp>(&pattern, base);
        }
        StringOrInit::Init(init) => init,
    };
    if base.is_some() {
        return Err(urlpattern::Error::BaseUrlWithInit);
    }
    let base_url = init
        .base_url
        .map(|base| Url::parse(&base))
        .transpose()
        .map_err(urlpattern::Error::Url)?;
    Ok(UrlPatternInit {
        protocol: init.protocol,
        username: init.username,
        password: init.password,
        hostname: init.hostname,
        port: init.port,
        pathname: init.pathname,
        search: init.search,
        hash: init.hash,
        base_url,
    })
}

fn cached_pattern<'js>(
    ctx: &Ctx<'js>,
    input: &str,
    base: Option<&str>,
    ignore_case: bool,
    runner: Runner<'js>,
) -> rquickjs::Result<Rc<Pattern>> {
    let key = pattern_key(input, base, ignore_case);
    if let Some(pattern) = PATTERNS.with_borrow(|patterns| patterns.get(&key).cloned()) {
        return Ok(pattern);
    }
    let pattern = Rc::new(compile_pattern(ctx, input, base, ignore_case, runner)?);
    PATTERNS.with_borrow_mut(|patterns| {
        if patterns.len() >= PATTERN_CACHE_LIMIT {
            patterns.clear();
        }
        patterns.insert(key, Rc::clone(&pattern));
    });
    Ok(pattern)
}

/// A pattern component's pattern string, and the regular expression that
/// matches it with its group names.
#[derive(IntoJs, JsType)]
struct PatternComponent {
    pattern: String,
    regexp: String,
    flags: String,
    groups: Vec<String>,
}

impl PatternComponent {
    fn new(
        ctx: &Ctx<'_>,
        component: &urlpattern::component::Component<EngineRegExp>,
    ) -> rquickjs::Result<Self> {
        let regexp = component
            .regexp
            .as_ref()
            .map_err(|error| Exception::throw_type(ctx, &error.to_string()))?;
        Ok(Self {
            pattern: component.pattern_string.clone(),
            regexp: regexp.source.clone(),
            flags: regexp.flags.clone(),
            groups: component.group_name_list.clone(),
        })
    }
}

/// A compiled URL pattern's components.
#[derive(IntoJs, JsType)]
#[qjs(rename_all = "camelCase")]
struct CompiledPattern {
    protocol: PatternComponent,
    username: PatternComponent,
    password: PatternComponent,
    hostname: PatternComponent,
    port: PatternComponent,
    pathname: PatternComponent,
    search: PatternComponent,
    hash: PatternComponent,
    #[qjs(rename = "hasRegExpGroups")]
    has_regexp_groups: bool,
}

#[host_function]
fn pattern_compile<'js>(
    ctx: Ctx<'js>,
    input: String,
    base: Option<String>,
    ignore_case: bool,
    runner: Runner<'js>,
) -> rquickjs::Result<CompiledPattern> {
    let pattern = cached_pattern(&ctx, &input, base.as_deref(), ignore_case, runner)?;
    Ok(CompiledPattern {
        protocol: PatternComponent::new(&ctx, &pattern.protocol)?,
        username: PatternComponent::new(&ctx, &pattern.username)?,
        password: PatternComponent::new(&ctx, &pattern.password)?,
        hostname: PatternComponent::new(&ctx, &pattern.hostname)?,
        port: PatternComponent::new(&ctx, &pattern.port)?,
        pathname: PatternComponent::new(&ctx, &pattern.pathname)?,
        search: PatternComponent::new(&ctx, &pattern.search)?,
        hash: PatternComponent::new(&ctx, &pattern.hash)?,
        has_regexp_groups: pattern.has_regexp_groups(),
    })
}

/// The component strings a URL or `URLPatternInit` input matches against.
#[derive(IntoJs, JsType)]
struct PatternInput {
    protocol: String,
    username: String,
    password: String,
    hostname: String,
    port: String,
    pathname: String,
    search: String,
    hash: String,
}

/// The components `input` matches against, or None when the input is not a
/// URL, which the spec treats as no match.
#[host_function]
fn pattern_match_input(
    ctx: Ctx<'_>,
    input: String,
    base: Option<String>,
) -> rquickjs::Result<Option<PatternInput>> {
    let parsed: StringOrInit = serde_json::from_str(&input).map_err(|error| {
        Exception::throw_type(&ctx, &format!("Invalid URLPattern input: {error}"))
    })?;
    let processed = quirks::process_match_input(parsed, base.as_deref())
        .map_err(|error| Exception::throw_type(&ctx, &error.to_string()))?;
    Ok(processed
        .and_then(|(input, _)| quirks::parse_match_input(input))
        .map(|input| PatternInput {
            protocol: input.protocol,
            username: input.username,
            password: input.password,
            hostname: input.hostname,
            port: input.port,
            pathname: input.pathname,
            search: input.search,
            hash: input.hash,
        }))
}

#[cfg(test)]
mod tests {
    use super::byte_offset;

    #[test]
    fn maps_utf16_offsets_to_byte_offsets() {
        let text = "a\u{1F600}b";
        let offsets: Vec<_> = (0..=5).map(|index| byte_offset(text, index)).collect();
        assert_eq!(offsets, [Some(0), Some(1), None, Some(5), Some(6), None]);
    }
}
