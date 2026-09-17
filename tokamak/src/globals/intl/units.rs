use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Read;
use std::rc::Rc;

use flate2::read::GzDecoder;
use icu::locale::Locale;
use icu::locale::fallback::LocaleFallbackConfig;
use rquickjs::{Ctx, Exception};
use serde_json::Value;

include!("units_data.rs");

thread_local! {
    static CACHE: RefCell<HashMap<usize, Rc<Value>>> = RefCell::new(HashMap::new());
}

pub(super) fn pattern(
    ctx: &Ctx<'_>,
    locale: &Locale,
    unit: &str,
    width: &str,
    plural: &str,
) -> rquickjs::Result<String> {
    let index = locale_index(locale)
        .ok_or_else(|| Exception::throw_internal(ctx, "Missing CLDR fallback locale"))?;
    let data = locale_data(ctx, index)?;
    let units = &data[width];
    if let Some(pattern) = plural_pattern(&units[unit], plural) {
        return Ok(pattern.to_owned());
    }
    let Some((numerator, denominator)) = unit.split_once("-per-") else {
        return Err(Exception::throw_range(ctx, "Invalid unit"));
    };
    let numerator = plural_pattern(&units[numerator], plural)
        .ok_or_else(|| Exception::throw_range(ctx, "Invalid numerator unit"))?;
    let denominator = &units[denominator];
    if let Some(per) = denominator["per"].as_str() {
        return Ok(per.replace("{0}", numerator));
    }
    let denominator = plural_pattern(denominator, "one")
        .ok_or_else(|| Exception::throw_range(ctx, "Invalid denominator unit"))?;
    let per = units["per"]["other"]
        .as_str()
        .ok_or_else(|| Exception::throw_internal(ctx, "Missing CLDR compound pattern"))?;
    Ok(per
        .replace("{0}", numerator)
        .replace("{1}", denominator.replace("{0}", "").trim()))
}

fn plural_pattern<'a>(patterns: &'a Value, plural: &str) -> Option<&'a str> {
    patterns
        .get(plural)
        .or_else(|| patterns.get("other"))?
        .as_str()
}

fn locale_index(locale: &Locale) -> Option<usize> {
    let config = super::provider::FALLBACKER.for_config(LocaleFallbackConfig::default());
    let mut iterator = config.fallback_for(locale.clone().into());
    loop {
        if iterator.get().is_unknown() {
            return LOCALES
                .binary_search_by_key(&"en", |&(name, _, _)| name)
                .ok();
        }
        let name = iterator.get().to_string();
        if let Ok(index) = LOCALES.binary_search_by_key(&name.as_str(), |&(name, _, _)| name) {
            return Some(index);
        }
        iterator.step();
    }
}

fn locale_data(ctx: &Ctx<'_>, index: usize) -> rquickjs::Result<Rc<Value>> {
    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(data) = cache.get(&index) {
            return Ok(Rc::clone(data));
        }
        let (_, offset, length) = LOCALES[index];
        let mut bytes = Vec::new();
        GzDecoder::new(&DATA[offset..offset + length])
            .read_to_end(&mut bytes)
            .map_err(|error| Exception::throw_internal(ctx, &error.to_string()))?;
        let data = Rc::new(
            serde_json::from_slice(&bytes)
                .map_err(|error| Exception::throw_internal(ctx, &error.to_string()))?,
        );
        if cache.len() == 8 {
            cache.clear();
        }
        cache.insert(index, Rc::clone(&data));
        Ok(data)
    })
}

pub(super) fn parts(pattern: &str, number: Vec<(String, String)>) -> Vec<(String, String)> {
    let Some((before, after)) = pattern.split_once("{0}") else {
        return vec![("unit".to_owned(), pattern.to_owned())];
    };
    let mut parts = Vec::new();
    let prefix = before.trim_end();
    append(&mut parts, "unit", prefix);
    append(&mut parts, "literal", &before[prefix.len()..]);
    parts.extend(number);
    let suffix = after.trim_start();
    append(&mut parts, "literal", &after[..after.len() - suffix.len()]);
    append(&mut parts, "unit", suffix);
    parts
}

fn append(parts: &mut Vec<(String, String)>, kind: &str, text: &str) {
    if !text.is_empty() {
        parts.push((kind.to_owned(), text.to_owned()));
    }
}

#[test]
fn generated_locale_patterns_are_complete() -> Result<(), Box<dyn std::error::Error>> {
    assert!(LOCALES.windows(2).all(|pair| pair[0].0 < pair[1].0));
    assert!(LOCALES.iter().any(|entry| entry.0 == "en"));
    let mut checked = std::collections::HashSet::new();
    for &(_, offset, length) in LOCALES {
        if !checked.insert(offset) {
            continue;
        }
        let bytes = DATA
            .get(offset..offset + length)
            .ok_or("invalid CLDR data range")?;
        let value: Value = serde_json::from_reader(GzDecoder::new(bytes))?;
        for width in ["long", "short", "narrow"] {
            assert!(value[width]["per"]["other"].as_str().is_some());
            assert!(value[width]["meter"]["other"].as_str().is_some());
        }
    }
    Ok(())
}
