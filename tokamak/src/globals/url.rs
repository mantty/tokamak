#![allow(clippy::needless_pass_by_value)]

use rquickjs::module::Exports;
use rquickjs::{Ctx, Exception, Function, Object, Value};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use url::Url;
use urlpattern::UrlPatternOptions;
use urlpattern::quirks::StringOrInit;

/// The compiled pattern type using the crate's default regex backend.
type Pattern = urlpattern::UrlPattern;

const PATTERN_CACHE_LIMIT: usize = 512;

thread_local! {
    static PATTERNS: RefCell<HashMap<String, Rc<Pattern>>> = RefCell::new(HashMap::new());
}

pub(super) const HOST_EXPORTS: &[&str] = &[
    "urlParse",
    "urlSetComponent",
    "urlEncodeParams",
    "urlDecodeParams",
    "urlPatternCompile",
    "urlPatternTest",
    "urlPatternExec",
];

pub(super) fn export_host_functions<'js>(
    ctx: &Ctx<'js>,
    exports: &Exports<'js>,
) -> rquickjs::Result<()> {
    exports.export("urlParse", Function::new(ctx.clone(), url_parse)?)?;
    exports.export(
        "urlSetComponent",
        Function::new(ctx.clone(), url_set_component)?,
    )?;
    exports.export(
        "urlEncodeParams",
        Function::new(ctx.clone(), encode_params)?,
    )?;
    exports.export(
        "urlDecodeParams",
        Function::new(ctx.clone(), decode_params)?,
    )?;
    exports.export(
        "urlPatternCompile",
        Function::new(ctx.clone(), pattern_compile)?,
    )?;
    exports.export("urlPatternTest", Function::new(ctx.clone(), pattern_test)?)?;
    exports.export("urlPatternExec", Function::new(ctx.clone(), pattern_exec)?)?;
    Ok(())
}

fn invalid_url(ctx: &Ctx<'_>, input: &str) -> rquickjs::Error {
    Exception::throw_type(ctx, &format!("Invalid URL: {input}"))
}

/// WHATWG URL accessor values, via the url crate's spec-shaped quirks module.
fn url_components<'js>(ctx: &Ctx<'js>, url: &Url) -> rquickjs::Result<Object<'js>> {
    let object = Object::new(ctx.clone())?;
    object.set("href", url::quirks::href(url))?;
    object.set("protocol", url::quirks::protocol(url))?;
    object.set("username", url::quirks::username(url))?;
    object.set("password", url::quirks::password(url))?;
    object.set("host", url::quirks::host(url))?;
    object.set("hostname", url::quirks::hostname(url))?;
    object.set("port", url::quirks::port(url))?;
    object.set("pathname", url::quirks::pathname(url))?;
    object.set("search", url::quirks::search(url))?;
    object.set("hash", url::quirks::hash(url))?;
    object.set("origin", url::quirks::origin(url))?;
    Ok(object)
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

fn url_parse(ctx: Ctx<'_>, input: String, base: Option<String>) -> rquickjs::Result<Object<'_>> {
    let url = parse_url(&ctx, &input, base.as_deref())?;
    url_components(&ctx, &url)
}

/// Applies a WHATWG component-set algorithm and returns the resulting
/// components. Per the spec, setter failures leave the URL unchanged; only
/// the href setter throws on invalid input.
fn url_set_component(
    ctx: Ctx<'_>,
    href: String,
    component: String,
    value: String,
) -> rquickjs::Result<Object<'_>> {
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
    url_components(&ctx, &url)
}

/// Serializes `[[name, value], ...]` JSON entries as application/x-www-form-urlencoded.
fn encode_params(ctx: Ctx<'_>, entries: String) -> rquickjs::Result<String> {
    let pairs: Vec<(String, String)> = serde_json::from_str(&entries)
        .map_err(|error| Exception::throw_type(&ctx, &error.to_string()))?;
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    serializer.extend_pairs(pairs);
    Ok(serializer.finish())
}

/// Parses application/x-www-form-urlencoded input into `[[name, value], ...]` JSON.
fn decode_params(ctx: Ctx<'_>, input: String) -> rquickjs::Result<String> {
    let pairs: Vec<(String, String)> = url::form_urlencoded::parse(input.as_bytes())
        .into_owned()
        .collect();
    serde_json::to_string(&pairs)
        .map_err(|error| Exception::throw_internal(&ctx, &error.to_string()))
}

fn pattern_key(input: &str, base: Option<&str>, ignore_case: bool) -> String {
    format!("{ignore_case}\u{1}{}\u{1}{input}", base.unwrap_or("\u{2}"))
}

fn compile_pattern(
    ctx: &Ctx<'_>,
    input: &str,
    base: Option<&str>,
    ignore_case: bool,
) -> rquickjs::Result<Pattern> {
    let input: StringOrInit = serde_json::from_str(input).map_err(|error| {
        Exception::throw_type(ctx, &format!("Invalid URLPattern input: {error}"))
    })?;
    let init = urlpattern::quirks::process_construct_pattern_input(input, base)
        .map_err(|error| Exception::throw_type(ctx, &error.to_string()))?;
    let options = UrlPatternOptions {
        ignore_case,
        ..UrlPatternOptions::default()
    };
    Pattern::parse(init, options).map_err(|error| Exception::throw_type(ctx, &error.to_string()))
}

fn cached_pattern(
    ctx: &Ctx<'_>,
    input: &str,
    base: Option<&str>,
    ignore_case: bool,
) -> rquickjs::Result<Rc<Pattern>> {
    let key = pattern_key(input, base, ignore_case);
    if let Some(pattern) = PATTERNS.with_borrow(|patterns| patterns.get(&key).cloned()) {
        return Ok(pattern);
    }
    let pattern = Rc::new(compile_pattern(ctx, input, base, ignore_case)?);
    PATTERNS.with_borrow_mut(|patterns| {
        if patterns.len() >= PATTERN_CACHE_LIMIT {
            patterns.clear();
        }
        patterns.insert(key, Rc::clone(&pattern));
    });
    Ok(pattern)
}

fn pattern_compile(
    ctx: Ctx<'_>,
    input: String,
    base: Option<String>,
    ignore_case: bool,
) -> rquickjs::Result<Object<'_>> {
    let pattern = cached_pattern(&ctx, &input, base.as_deref(), ignore_case)?;
    let object = Object::new(ctx.clone())?;
    object.set("protocol", pattern.protocol())?;
    object.set("username", pattern.username())?;
    object.set("password", pattern.password())?;
    object.set("hostname", pattern.hostname())?;
    object.set("port", pattern.port())?;
    object.set("pathname", pattern.pathname())?;
    object.set("search", pattern.search())?;
    object.set("hash", pattern.hash())?;
    object.set("hasRegExpGroups", pattern.has_regexp_groups())?;
    Ok(object)
}

/// Processes a match input (URL string or `URLPatternInit` JSON). Returns None
/// when the input fails to parse as a URL, which the spec treats as no match.
fn match_input(
    ctx: &Ctx<'_>,
    input: &str,
    base: Option<&str>,
) -> rquickjs::Result<Option<urlpattern::UrlPatternMatchInput>> {
    let parsed: StringOrInit = serde_json::from_str(input).map_err(|error| {
        Exception::throw_type(ctx, &format!("Invalid URLPattern input: {error}"))
    })?;
    match urlpattern::quirks::process_match_input(parsed, base) {
        Ok(processed) => Ok(processed.map(|(input, _)| input)),
        Err(error) => Err(Exception::throw_type(ctx, &error.to_string())),
    }
}

fn pattern_test(
    ctx: Ctx<'_>,
    pattern: String,
    pattern_base: Option<String>,
    ignore_case: bool,
    input: String,
    base: Option<String>,
) -> rquickjs::Result<bool> {
    let compiled = cached_pattern(&ctx, &pattern, pattern_base.as_deref(), ignore_case)?;
    let Some(input) = match_input(&ctx, &input, base.as_deref())? else {
        return Ok(false);
    };
    compiled
        .test(input)
        .map_err(|error| Exception::throw_type(&ctx, &error.to_string()))
}

fn pattern_exec(
    ctx: Ctx<'_>,
    pattern: String,
    pattern_base: Option<String>,
    ignore_case: bool,
    input: String,
    base: Option<String>,
) -> rquickjs::Result<Option<Object<'_>>> {
    let compiled = cached_pattern(&ctx, &pattern, pattern_base.as_deref(), ignore_case)?;
    let Some(input) = match_input(&ctx, &input, base.as_deref())? else {
        return Ok(None);
    };
    let result = compiled
        .exec(input)
        .map_err(|error| Exception::throw_type(&ctx, &error.to_string()))?;
    result
        .map(|result| match_object(&ctx, &compiled, &result))
        .transpose()
}

fn match_object<'js>(
    ctx: &Ctx<'js>,
    compiled: &Pattern,
    result: &urlpattern::UrlPatternResult,
) -> rquickjs::Result<Object<'js>> {
    let object = Object::new(ctx.clone())?;
    let components = [
        ("protocol", &compiled.protocol, &result.protocol),
        ("username", &compiled.username, &result.username),
        ("password", &compiled.password, &result.password),
        ("hostname", &compiled.hostname, &result.hostname),
        ("port", &compiled.port, &result.port),
        ("pathname", &compiled.pathname, &result.pathname),
        ("search", &compiled.search, &result.search),
        ("hash", &compiled.hash, &result.hash),
    ];
    for (name, component, value) in components {
        object.set(name, component_result(ctx, component, value)?)?;
    }
    Ok(object)
}

/// Builds `{input, groups}` for one component, with group keys in pattern
/// order and unmatched groups present as undefined, per the `URLPattern` spec.
fn component_result<'js, R: urlpattern::regexp::RegExp>(
    ctx: &Ctx<'js>,
    component: &urlpattern::component::Component<R>,
    result: &urlpattern::UrlPatternComponentResult,
) -> rquickjs::Result<Object<'js>> {
    let object = Object::new(ctx.clone())?;
    object.set("input", result.input.as_str())?;
    let groups = Object::new(ctx.clone())?;
    for name in &component.group_name_list {
        match result.groups.get(name) {
            Some(Some(matched)) => groups.set(name.as_str(), matched.as_str())?,
            _ => groups.set(name.as_str(), Value::new_undefined(ctx.clone()))?,
        }
    }
    object.set("groups", groups)?;
    Ok(object)
}
