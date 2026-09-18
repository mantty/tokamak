#![allow(clippy::needless_pass_by_value)]

mod provider;
mod units;
include!("intl/plurals_data.rs");

use provider::{PROVIDER, resolve};

use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::fmt;
use std::thread::LocalKey;

use fixed_decimal::{
    Decimal as FixedDecimal, Sign, SignDisplay, SignedRoundingMode, UnsignedRoundingMode,
};
use icu::calendar::{Date, Iso};
use icu::collator::preferences::{CollationCaseFirst, CollationNumericOrdering};
use icu::collator::{Collator, CollatorPreferences};
use icu::datetime::fieldsets::builder::{DateFields, FieldSetBuilder, ZoneStyle};
use icu::datetime::fieldsets::enums::CompositeFieldSet;
use icu::datetime::options::{Length, TimePrecision, YearStyle};
use icu::datetime::preferences::HourCycle;
use icu::datetime::{DateTimeFormatter, DateTimeFormatterPreferences};
use icu::decimal::options::{
    CompactDecimalFormatterOptions, DecimalFormatterOptions, GroupingStrategy,
};
use icu::decimal::{CompactDecimalFormatter, DecimalFormatter};
use icu::experimental::dimension::currency::CurrencyType;
use icu::experimental::dimension::currency::formatter::{
    CurrencyFormatter, CurrencyFormatterPreferences,
};
use icu::experimental::dimension::currency::options::{CurrencyFormatterOptions, CurrencyUsage};
use icu::experimental::dimension::percent::formatter::{
    PercentFormatter, PercentFormatterPreferences,
};
use icu::experimental::dimension::percent::options::PercentFormatterOptions;
use icu::experimental::relativetime::options::Numeric as RelativeNumeric;
use icu::experimental::relativetime::{RelativeTimeFormatter, RelativeTimeFormatterOptions};
use icu::list::ListFormatter;
use icu::list::options::{ListFormatterOptions, ListLength};
use icu::locale::names::{
    DisplayNamesPreferences, LanguageIdentifierDisplayName, LanguageIdentifierDisplayNameOptions,
    RegionDisplayName, ScriptDisplayName,
};
use icu::locale::subtags::{Language, Region, Script};
use icu::locale::{Locale, LocaleCanonicalizer, LocaleExpander};
use icu::plurals::{
    PluralCategory, PluralRuleType, PluralRules, PluralRulesOptions, PluralRulesWithRanges,
};
use icu::segmenter::options::{SentenceBreakOptions, WordBreakOptions};
use icu::segmenter::{GraphemeClusterSegmenter, SentenceSegmenter, WordSegmenter};
use icu::time::ZonedDateTime;
use icu::time::zone::models::AtTime;
use icu::time::zone::{TimeZoneInfo, UtcOffset, ZoneNameTimestamp};
use jiff::Timestamp;
use rquickjs::module::Exports;
use rquickjs::{Ctx, Exception, Function};
use serde_json::Value;
use writeable::{Part, PartsWrite, Writeable};

pub(super) const HOST_EXPORTS: &[&str] = &[
    "intlCanonicalLocales",
    "intlDateTime",
    "intlDateTimeParts",
    "intlNumber",
    "intlNumberParts",
    "intlPlural",
    "intlPluralRange",
    "intlPluralCategories",
    "intlList",
    "intlListParts",
    "intlRelative",
    "intlRelativeParts",
    "intlCollatorCompare",
    "intlSegment",
    "intlDisplayName",
    "intlLocaleInfo",
];

pub(super) fn export_host_functions<'js>(
    ctx: &Ctx<'js>,
    exports: &Exports<'js>,
) -> rquickjs::Result<()> {
    let export = |name: &str, function: Function<'js>| exports.export(name, function);
    export(
        "intlCanonicalLocales",
        Function::new(ctx.clone(), canonical_locales)?,
    )?;
    export("intlDateTime", Function::new(ctx.clone(), date_time)?)?;
    export(
        "intlDateTimeParts",
        Function::new(ctx.clone(), date_time_parts)?,
    )?;
    export("intlNumber", Function::new(ctx.clone(), number)?)?;
    export("intlNumberParts", Function::new(ctx.clone(), number_parts)?)?;
    export("intlPlural", Function::new(ctx.clone(), plural)?)?;
    export("intlPluralRange", Function::new(ctx.clone(), plural_range)?)?;
    export(
        "intlPluralCategories",
        Function::new(ctx.clone(), plural_categories)?,
    )?;
    export("intlList", Function::new(ctx.clone(), list)?)?;
    export("intlListParts", Function::new(ctx.clone(), list_parts)?)?;
    export("intlRelative", Function::new(ctx.clone(), relative)?)?;
    export(
        "intlRelativeParts",
        Function::new(ctx.clone(), relative_parts)?,
    )?;
    export(
        "intlCollatorCompare",
        Function::new(ctx.clone(), collator_compare)?,
    )?;
    export("intlSegment", Function::new(ctx.clone(), segment)?)?;
    export("intlDisplayName", Function::new(ctx.clone(), display_name)?)?;
    export("intlLocaleInfo", Function::new(ctx.clone(), locale_info)?)?;
    Ok(())
}

// Requests run on dedicated threads, so per-thread caches need no locking.
// Keys embed the locale list and options JSON exactly as received from JS.
const CACHE_LIMIT: usize = 64;

thread_local! {
    static COLLATORS: RefCell<HashMap<String, Collator>> =
        RefCell::new(HashMap::new());
    static DATE_TIME_FORMATTERS: RefCell<HashMap<String, DateTimeFormatter<CompositeFieldSet>>> =
        RefCell::new(HashMap::new());
    static DECIMAL_FORMATTERS: RefCell<HashMap<String, DecimalFormatter>> =
        RefCell::new(HashMap::new());
}

fn with_cached<F, T>(
    cache: &'static LocalKey<RefCell<HashMap<String, F>>>,
    key: &str,
    build: impl FnOnce() -> rquickjs::Result<F>,
    apply: impl FnOnce(&F) -> T,
) -> rquickjs::Result<T> {
    cache.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.len() >= CACHE_LIMIT && !cache.contains_key(key) {
            cache.clear();
        }
        let value = match cache.entry(key.to_owned()) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => entry.insert(build()?),
        };
        Ok(apply(value))
    })
}

fn with_decimal_formatter<T>(
    ctx: &Ctx<'_>,
    locale: &Locale,
    strategy: GroupingStrategy,
    apply: impl FnOnce(&DecimalFormatter) -> T,
) -> rquickjs::Result<T> {
    let key = format!("{locale}\u{1}{strategy:?}");
    with_cached(
        &DECIMAL_FORMATTERS,
        &key,
        || {
            DecimalFormatter::try_new_unstable(
                &*PROVIDER,
                locale.clone().into(),
                DecimalFormatterOptions::from(strategy),
            )
            .map_err(|error| Exception::throw_range(ctx, &error.to_string()))
        },
        apply,
    )
}

pub(super) fn canonical_locales(ctx: Ctx<'_>, input: String) -> rquickjs::Result<String> {
    let values: Vec<String> = serde_json::from_str(&input)
        .map_err(|error| Exception::throw_type(&ctx, &error.to_string()))?;
    let canonicalizer = LocaleCanonicalizer::try_new_extended_unstable(&*PROVIDER)
        .map_err(|error| Exception::throw_internal(&ctx, &error.to_string()))?;
    let mut output = Vec::with_capacity(values.len());
    for value in values {
        let mut locale = value
            .parse::<Locale>()
            .map_err(|error| Exception::throw_range(&ctx, &error.to_string()))?;
        canonicalizer.canonicalize(&mut locale);
        let value = locale.to_string();
        if !output.contains(&value) {
            output.push(value);
        }
    }
    serde_json::to_string(&output)
        .map_err(|error| Exception::throw_internal(&ctx, &error.to_string()))
}

pub(super) fn date_time(
    ctx: Ctx<'_>,
    milliseconds: f64,
    locales: String,
    options: String,
) -> rquickjs::Result<String> {
    format_date_time(&ctx, milliseconds, &locales, &options, false)
}

pub(super) fn date_time_parts(
    ctx: Ctx<'_>,
    milliseconds: f64,
    locales: String,
    options: String,
) -> rquickjs::Result<String> {
    format_date_time(&ctx, milliseconds, &locales, &options, true)
}

pub(super) fn number(
    ctx: Ctx<'_>,
    value: f64,
    locales: String,
    options: String,
) -> rquickjs::Result<String> {
    let locale = resolved_locale(&ctx, &locales)?;
    let options: Value = serde_json::from_str(&options)
        .map_err(|error| Exception::throw_type(&ctx, &error.to_string()))?;
    format_number(&ctx, value, &locale, &options)
}

pub(super) fn number_parts(
    ctx: Ctx<'_>,
    value: f64,
    locales: String,
    options: String,
) -> rquickjs::Result<String> {
    let locale = resolved_locale(&ctx, &locales)?;
    let options: Value = serde_json::from_str(&options)
        .map_err(|error| Exception::throw_type(&ctx, &error.to_string()))?;
    format_number_parts(&ctx, value, &locale, &options)
}

pub(super) fn plural(
    ctx: Ctx<'_>,
    value: f64,
    locales: String,
    kind: String,
) -> rquickjs::Result<String> {
    if !value.is_finite() {
        return Ok("other".to_owned());
    }
    let rules = plural_rules(&ctx, &locales, &kind)?;
    let decimal = plural_operand(&ctx, value)?;
    Ok(plural_category(rules.category_for(&decimal)).to_owned())
}

pub(super) fn plural_range(
    ctx: Ctx<'_>,
    start: f64,
    end: f64,
    locales: String,
    kind: String,
) -> rquickjs::Result<String> {
    if !start.is_finite() || !end.is_finite() {
        return Ok("other".to_owned());
    }
    let locale = resolved_locale(&ctx, &locales)?;
    let options = plural_rules_options(&kind);
    let rules = PluralRulesWithRanges::try_new_unstable(
        &*PROVIDER,
        plural_locale(&ctx, &locale)?.into(),
        options,
    )
    .map_err(|error| Exception::throw_range(&ctx, &error.to_string()))?;
    let start = plural_operand(&ctx, start)?;
    let end = plural_operand(&ctx, end)?;
    Ok(plural_category(rules.category_for_range(&start, &end)).to_owned())
}

pub(super) fn plural_categories(
    ctx: Ctx<'_>,
    locales: String,
    kind: String,
) -> rquickjs::Result<String> {
    let rules = plural_rules(&ctx, &locales, &kind)?;
    let values = rules.categories().map(plural_category).collect::<Vec<_>>();
    serde_json::to_string(&values)
        .map_err(|error| Exception::throw_internal(&ctx, &error.to_string()))
}

pub(super) fn list(
    ctx: Ctx<'_>,
    values: String,
    locales: String,
    options: String,
) -> rquickjs::Result<String> {
    let values: Vec<String> = serde_json::from_str(&values)
        .map_err(|error| Exception::throw_type(&ctx, &error.to_string()))?;
    let locale = resolved_locale(&ctx, &locales)?;
    let options: Value = serde_json::from_str(&options)
        .map_err(|error| Exception::throw_type(&ctx, &error.to_string()))?;
    let formatter = list_formatter(&ctx, locale, &options)?;
    Ok(formatter.format_to_string(values.iter().map(String::as_str)))
}

pub(super) fn list_parts(
    ctx: Ctx<'_>,
    values: String,
    locales: String,
    options: String,
) -> rquickjs::Result<String> {
    let values: Vec<String> = serde_json::from_str(&values)
        .map_err(|error| Exception::throw_type(&ctx, &error.to_string()))?;
    let locale = resolved_locale(&ctx, &locales)?;
    let options: Value = serde_json::from_str(&options)
        .map_err(|error| Exception::throw_type(&ctx, &error.to_string()))?;
    let formatter = list_formatter(&ctx, locale, &options)?;
    let mut collector = PartsCollector::default();
    formatter
        .format(values.iter().map(String::as_str))
        .write_to_parts(&mut collector)
        .map_err(|_| Exception::throw_internal(&ctx, "Failed to format list parts"))?;
    serde_json::to_string(&collector.parts)
        .map_err(|error| Exception::throw_internal(&ctx, &error.to_string()))
}

pub(super) fn relative(
    ctx: Ctx<'_>,
    value: f64,
    unit: String,
    locales: String,
    options: String,
) -> rquickjs::Result<String> {
    let (locale, options) = relative_inputs(&ctx, value, &locales, &options)?;
    format_relative(&ctx, value, &unit, &locale, &options)
}

fn relative_inputs(
    ctx: &Ctx<'_>,
    value: f64,
    locales: &str,
    options: &str,
) -> rquickjs::Result<(Locale, Value)> {
    if !value.is_finite() {
        return Err(Exception::throw_range(ctx, "Invalid relative time value"));
    }
    let locale = resolved_locale(ctx, locales)?;
    let options = serde_json::from_str(options)
        .map_err(|error| Exception::throw_type(ctx, &error.to_string()))?;
    Ok((locale, options))
}

fn format_relative(
    ctx: &Ctx<'_>,
    value: f64,
    unit: &str,
    locale: &Locale,
    options: &Value,
) -> rquickjs::Result<String> {
    let mut formatter_options = RelativeTimeFormatterOptions::default();
    formatter_options.numeric = if options.get("numeric").and_then(Value::as_str) == Some("auto") {
        RelativeNumeric::Auto
    } else {
        RelativeNumeric::Always
    };
    macro_rules! try_new {
        ($constructor:ident) => {
            RelativeTimeFormatter::$constructor(
                &*PROVIDER,
                locale.clone().into(),
                formatter_options,
            )
        };
    }
    let formatter: RelativeTimeFormatter = match (
        unit,
        options
            .get("style")
            .and_then(Value::as_str)
            .unwrap_or("long"),
    ) {
        ("second", "short") => try_new!(try_new_short_second_unstable),
        ("second", "narrow") => try_new!(try_new_narrow_second_unstable),
        ("minute", "short") => try_new!(try_new_short_minute_unstable),
        ("minute", "narrow") => try_new!(try_new_narrow_minute_unstable),
        ("hour", "short") => try_new!(try_new_short_hour_unstable),
        ("hour", "narrow") => try_new!(try_new_narrow_hour_unstable),
        ("day", "short") => try_new!(try_new_short_day_unstable),
        ("day", "narrow") => try_new!(try_new_narrow_day_unstable),
        ("week", "short") => try_new!(try_new_short_week_unstable),
        ("week", "narrow") => try_new!(try_new_narrow_week_unstable),
        ("month", "short") => try_new!(try_new_short_month_unstable),
        ("month", "narrow") => try_new!(try_new_narrow_month_unstable),
        ("quarter", "short") => try_new!(try_new_short_quarter_unstable),
        ("quarter", "narrow") => try_new!(try_new_narrow_quarter_unstable),
        ("year", "short") => try_new!(try_new_short_year_unstable),
        ("year", "narrow") => try_new!(try_new_narrow_year_unstable),
        ("second", _) => try_new!(try_new_long_second_unstable),
        ("minute", _) => try_new!(try_new_long_minute_unstable),
        ("hour", _) => try_new!(try_new_long_hour_unstable),
        ("day", _) => try_new!(try_new_long_day_unstable),
        ("week", _) => try_new!(try_new_long_week_unstable),
        ("month", _) => try_new!(try_new_long_month_unstable),
        ("quarter", _) => try_new!(try_new_long_quarter_unstable),
        ("year", _) => try_new!(try_new_long_year_unstable),
        _ => return Err(Exception::throw_range(ctx, "Invalid relative time unit")),
    }
    .map_err(|error| Exception::throw_range(ctx, &error.to_string()))?;
    let decimal = value
        .to_string()
        .parse::<icu::decimal::input::Decimal>()
        .map_err(|error| Exception::throw_range(ctx, &error.to_string()))?;
    Ok(formatter.format(decimal).to_string())
}

pub(super) fn relative_parts(
    ctx: Ctx<'_>,
    value: f64,
    unit: String,
    locales: String,
    options: String,
) -> rquickjs::Result<String> {
    let (locale, options) = relative_inputs(&ctx, value, &locales, &options)?;
    let formatted = format_relative(&ctx, value, &unit, &locale, &options)?;
    let absolute = plural_operand(&ctx, value.abs())?;
    let number = with_decimal_formatter(&ctx, &locale, GroupingStrategy::Auto, |formatter| {
        formatter.format_to_string(&absolute)
    })?;
    let Some(index) = formatted.find(&number) else {
        return serde_json::to_string(&[
            serde_json::json!({ "type": "literal", "value": formatted }),
        ])
        .map_err(|error| Exception::throw_internal(&ctx, &error.to_string()));
    };
    let mut parts = Vec::new();
    if index > 0 {
        parts.push(("literal", formatted[..index].to_owned()));
    }
    let number_len = number.len();
    parts.push((
        if value.fract() == 0.0 {
            "integer"
        } else {
            "fraction"
        },
        number,
    ));
    let suffix = &formatted[index + number_len..];
    if !suffix.is_empty() {
        parts.push(("literal", suffix.to_owned()));
    }
    let parts = parts
        .into_iter()
        .map(|(kind, value)| {
            if kind == "integer" || kind == "fraction" {
                serde_json::json!({ "type": kind, "value": value, "unit": unit })
            } else {
                serde_json::json!({ "type": kind, "value": value })
            }
        })
        .collect::<Vec<_>>();
    serde_json::to_string(&parts)
        .map_err(|error| Exception::throw_internal(&ctx, &error.to_string()))
}

pub(super) fn collator_compare(
    ctx: Ctx<'_>,
    left: String,
    right: String,
    locales: String,
    options: String,
) -> rquickjs::Result<i32> {
    let key = format!("{locales}\u{1}{options}");
    with_cached(
        &COLLATORS,
        &key,
        || collator(&ctx, &locales, &options),
        |collator| match collator.as_borrowed().compare(&left, &right) {
            Ordering::Less => -1,
            Ordering::Equal => 0,
            Ordering::Greater => 1,
        },
    )
}

fn collator(ctx: &Ctx<'_>, locales: &str, options: &str) -> rquickjs::Result<Collator> {
    let locale = resolved_locale(ctx, locales)?;
    let options: Value = serde_json::from_str(options)
        .map_err(|error| Exception::throw_type(ctx, &error.to_string()))?;
    let mut prefs = CollatorPreferences::from(locale);
    prefs.numeric_ordering = match options.get("numeric").and_then(Value::as_bool) {
        Some(true) => Some(CollationNumericOrdering::True),
        Some(false) => Some(CollationNumericOrdering::False),
        None => None,
    };
    prefs.case_first = match options.get("caseFirst").and_then(Value::as_str) {
        Some("upper") => Some(CollationCaseFirst::Upper),
        Some("lower") => Some(CollationCaseFirst::Lower),
        Some("false") => Some(CollationCaseFirst::False),
        _ => None,
    };
    let mut collator_options = icu::collator::options::CollatorOptions::default();
    collator_options.strength = match options.get("sensitivity").and_then(Value::as_str) {
        Some("base" | "case") => Some(icu::collator::options::Strength::Primary),
        Some("accent") => Some(icu::collator::options::Strength::Secondary),
        Some("variant") => Some(icu::collator::options::Strength::Tertiary),
        _ => None,
    };
    if options.get("sensitivity").and_then(Value::as_str) == Some("case") {
        collator_options.case_level = Some(icu::collator::options::CaseLevel::On);
    }
    if options.get("ignorePunctuation").and_then(Value::as_bool) == Some(true) {
        collator_options.alternate_handling =
            Some(icu::collator::options::AlternateHandling::Shifted);
        collator_options.max_variable = Some(icu::collator::options::MaxVariable::Punctuation);
    }
    Collator::try_new_unstable(&*PROVIDER, prefs, collator_options)
        .map_err(|error| Exception::throw_range(ctx, &error.to_string()))
}

pub(super) fn segment(
    ctx: Ctx<'_>,
    text: String,
    locales: String,
    granularity: String,
) -> rquickjs::Result<String> {
    let _locale = first_locale(&ctx, &locales)?;
    let range = |error: icu_provider::DataError| Exception::throw_range(&ctx, &error.to_string());
    let (mut boundaries, word_types) = match granularity.as_str() {
        "grapheme" => {
            let segmenter =
                GraphemeClusterSegmenter::try_new_unstable(&*PROVIDER).map_err(range)?;
            (
                segmenter
                    .as_borrowed()
                    .segment_str(&text)
                    .collect::<Vec<_>>(),
                None,
            )
        }
        "sentence" => {
            let segmenter =
                SentenceSegmenter::try_new_unstable(&*PROVIDER, SentenceBreakOptions::default())
                    .map_err(range)?;
            (
                segmenter
                    .as_borrowed()
                    .segment_str(&text)
                    .collect::<Vec<_>>(),
                None,
            )
        }
        "word" => {
            let segmenter = WordSegmenter::try_new_for_non_complex_scripts_unstable(
                &*PROVIDER,
                WordBreakOptions::default(),
            )
            .map_err(range)?;
            let pairs = segmenter
                .as_borrowed()
                .segment_str(&text)
                .iter_with_word_type()
                .collect::<Vec<_>>();
            let boundaries = pairs.iter().map(|&(boundary, _)| boundary).collect();
            (boundaries, Some(pairs))
        }
        _ => {
            return Err(Exception::throw_range(
                &ctx,
                "Invalid segmenter granularity",
            ));
        }
    };
    if boundaries.is_empty() {
        boundaries.push(0);
    }
    let values = boundaries
        .windows(2)
        .enumerate()
        .map(|(index, window)| {
            let start = window[0];
            let end = window[1];
            let mut value = serde_json::json!({
                "segment": &text[start..end],
                "index": text[..start].encode_utf16().count(),
                "input": &text,
            });
            if let Some(word_types) = &word_types {
                let is_word_like = word_types
                    .get(index + 1)
                    .is_some_and(|(_, word_type)| word_type.is_word_like());
                value["isWordLike"] = Value::Bool(is_word_like);
            }
            value
        })
        .collect::<Vec<_>>();
    serde_json::to_string(&values)
        .map_err(|error| Exception::throw_internal(&ctx, &error.to_string()))
}

pub(super) fn display_name(
    ctx: Ctx<'_>,
    code: String,
    locales: String,
    options: String,
) -> rquickjs::Result<String> {
    let locale = resolved_locale(&ctx, &locales)?;
    let options: Value = serde_json::from_str(&options)
        .map_err(|error| Exception::throw_type(&ctx, &error.to_string()))?;
    let kind = options
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("language");
    let style = options
        .get("style")
        .and_then(Value::as_str)
        .unwrap_or("long");
    let prefs = DisplayNamesPreferences::from(locale.clone());
    let value = match kind {
        "language" => {
            let language = code
                .parse::<Language>()
                .map_err(|error| Exception::throw_range(&ctx, &error.to_string()))?;
            let id = language.into();
            let language_display = if style == "short" {
                LanguageIdentifierDisplayName::try_new_short_light_unstable(
                    &*PROVIDER,
                    prefs,
                    id,
                    LanguageIdentifierDisplayNameOptions::default(),
                )
            } else {
                LanguageIdentifierDisplayName::try_new_long_light_unstable(
                    &*PROVIDER,
                    prefs,
                    id,
                    LanguageIdentifierDisplayNameOptions::default(),
                )
            }
            .map_err(|error| Exception::throw_range(&ctx, &error.to_string()))?;
            language_display.as_borrowed().to_string()
        }
        "region" => {
            let region = code
                .parse::<Region>()
                .map_err(|error| Exception::throw_range(&ctx, &error.to_string()))?;
            if style == "short" {
                RegionDisplayName::try_new_short_light_unstable(&*PROVIDER, prefs, region)
                    .map_err(|error| Exception::throw_range(&ctx, &error.to_string()))?
                    .to_string()
            } else {
                RegionDisplayName::try_new_light_unstable(&*PROVIDER, prefs, region)
                    .map_err(|error| Exception::throw_range(&ctx, &error.to_string()))?
                    .to_string()
            }
        }
        "script" => {
            let script = code
                .parse::<Script>()
                .map_err(|error| Exception::throw_range(&ctx, &error.to_string()))?;
            ScriptDisplayName::try_new_light_unstable(&*PROVIDER, prefs, script)
                .map_err(|error| Exception::throw_range(&ctx, &error.to_string()))?
                .to_string()
        }
        "calendar" => calendar_display_name(&code, &locale),
        "currency" => currency_display_name(&code, &locale),
        "dateTimeField" => date_time_field_display_name(&code, &locale),
        _ => code,
    };
    Ok(value)
}

pub(super) fn locale_info(ctx: Ctx<'_>, tag: String, _options: String) -> rquickjs::Result<String> {
    let mut locale = tag
        .parse::<Locale>()
        .map_err(|error| Exception::throw_range(&ctx, &error.to_string()))?;
    LocaleCanonicalizer::try_new_extended_unstable(&*PROVIDER)
        .map_err(|error| Exception::throw_internal(&ctx, &error.to_string()))?
        .canonicalize(&mut locale);
    let mut maximum = locale.clone();
    let expander = LocaleExpander::try_new_extended_unstable(&*PROVIDER)
        .map_err(|error| Exception::throw_internal(&ctx, &error.to_string()))?;
    expander.maximize(&mut maximum.id);
    let mut minimum = maximum.clone();
    expander.minimize(&mut minimum.id);
    let value = serde_json::json!({
        "string": locale.to_string(),
        "baseName": locale.id.to_string(),
        "language": locale.id.language.to_string(),
        "script": locale.id.script.map(|value| value.to_string()),
        "region": locale.id.region.map(|value| value.to_string()),
        "calendar": locale_keyword(&locale, "ca").unwrap_or_else(|| "gregory".to_owned()),
        "numberingSystem": locale_keyword(&locale, "nu").unwrap_or_else(|| "latn".to_owned()),
        "hourCycle": locale_keyword(&locale, "hc"),
        "maximize": maximum.to_string(),
        "minimize": minimum.to_string(),
    });
    serde_json::to_string(&value)
        .map_err(|error| Exception::throw_internal(&ctx, &error.to_string()))
}

fn first_locale(ctx: &Ctx<'_>, locales: &str) -> rquickjs::Result<String> {
    let values: Vec<String> = serde_json::from_str(locales)
        .map_err(|error| Exception::throw_type(ctx, &error.to_string()))?;
    let value = values
        .into_iter()
        .next()
        .unwrap_or_else(|| "en-US".to_owned());
    Ok(value)
}

// Parses the first requested locale and maps it onto the bundled data set,
// falling unsupported locales back to `en-US` exactly as workerd does.
fn resolved_locale(ctx: &Ctx<'_>, locales: &str) -> rquickjs::Result<Locale> {
    let locale = first_locale(ctx, locales)?
        .parse::<Locale>()
        .map_err(|error| Exception::throw_range(ctx, &error.to_string()))?;
    Ok(resolve(locale))
}

fn plural_operand(ctx: &Ctx<'_>, value: f64) -> rquickjs::Result<FixedDecimal> {
    value
        .to_string()
        .parse::<FixedDecimal>()
        .map_err(|error| Exception::throw_range(ctx, &error.to_string()))
}

fn plural_locale(ctx: &Ctx<'_>, locale: &Locale) -> rquickjs::Result<Locale> {
    // Supplemental plural data uses subtag lookup, not display-data parent
    // overrides (which would route sr-Latn to root instead of Serbian).
    let name = locale.id.to_string();
    let mut candidate = name.as_str();
    loop {
        if candidate != "und" && PLURAL_LOCALES.binary_search(&candidate).is_ok() {
            return candidate
                .parse::<Locale>()
                .map_err(|error| Exception::throw_internal(ctx, &error.to_string()));
        }
        let Some((parent, _)) = candidate.rsplit_once('-') else {
            return Ok(icu::locale::locale!("en"));
        };
        candidate = parent;
    }
}

fn plural_rules(ctx: &Ctx<'_>, locales: &str, kind: &str) -> rquickjs::Result<PluralRules> {
    let locale = resolved_locale(ctx, locales)?;
    PluralRules::try_new_unstable(
        &*PROVIDER,
        plural_locale(ctx, &locale)?.into(),
        plural_rules_options(kind),
    )
    .map_err(|error| Exception::throw_range(ctx, &error.to_string()))
}

fn plural_rules_options(kind: &str) -> PluralRulesOptions {
    PluralRulesOptions::default().with_type(if kind == "ordinal" {
        PluralRuleType::Ordinal
    } else {
        PluralRuleType::Cardinal
    })
}

fn plural_category(category: PluralCategory) -> &'static str {
    match category {
        PluralCategory::Zero => "zero",
        PluralCategory::One => "one",
        PluralCategory::Two => "two",
        PluralCategory::Few => "few",
        PluralCategory::Many => "many",
        PluralCategory::Other => "other",
    }
}

fn list_formatter(
    ctx: &Ctx<'_>,
    locale: Locale,
    options: &Value,
) -> rquickjs::Result<ListFormatter> {
    let formatter_options = ListFormatterOptions::default().with_length(list_length(options));
    match options
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("conjunction")
    {
        "disjunction" => {
            ListFormatter::try_new_or_unstable(&*PROVIDER, locale.into(), formatter_options)
        }
        "unit" => {
            ListFormatter::try_new_unit_unstable(&*PROVIDER, locale.into(), formatter_options)
        }
        _ => ListFormatter::try_new_and_unstable(&*PROVIDER, locale.into(), formatter_options),
    }
    .map_err(|error| Exception::throw_range(ctx, &error.to_string()))
}

fn list_length(options: &Value) -> ListLength {
    match options.get("style").and_then(Value::as_str) {
        Some("narrow") => ListLength::Narrow,
        Some("short") => ListLength::Short,
        _ => ListLength::Wide,
    }
}

fn locale_keyword(locale: &Locale, name: &str) -> Option<String> {
    let key = name.parse().ok()?;
    locale
        .extensions
        .unicode
        .keywords
        .get(&key)
        .map(ToString::to_string)
        .filter(|value| !value.is_empty())
}

// icu4x 2.3 exposes no display-name API for calendars, currencies, or
// date-time fields; en keeps a small table, other locales return the code.
fn calendar_display_name(code: &str, locale: &Locale) -> String {
    if locale.id.language.as_str() == "en" {
        match code {
            "gregory" => "Gregorian Calendar",
            "buddhist" => "Buddhist Calendar",
            "japanese" => "Japanese Calendar",
            "islamic" => "Islamic Calendar",
            _ => code,
        }
        .to_owned()
    } else {
        code.to_owned()
    }
}

fn currency_display_name(code: &str, locale: &Locale) -> String {
    if locale.id.language.as_str() == "en" {
        match code {
            "GBP" => "British Pound",
            "USD" => "US Dollar",
            "EUR" => "Euro",
            "JPY" => "Japanese Yen",
            _ => code,
        }
        .to_owned()
    } else {
        code.to_owned()
    }
}

fn date_time_field_display_name(code: &str, locale: &Locale) -> String {
    if locale.id.language.as_str() == "en" {
        match code {
            "era" => "era",
            "year" => "year",
            "quarter" => "quarter",
            "month" => "month",
            "weekOfYear" | "weekOfMonth" => "week",
            "day" => "day",
            "dayOfWeek" => "day of the week",
            "dayperiod" | "dayPeriod" => "AM/PM",
            "hour" => "hour",
            "minute" => "minute",
            "second" => "second",
            "zone" => "time zone",
            _ => code,
        }
        .to_owned()
    } else {
        code.to_owned()
    }
}

fn format_date_time(
    ctx: &Ctx<'_>,
    milliseconds: f64,
    locales: &str,
    options_json: &str,
    parts: bool,
) -> rquickjs::Result<String> {
    if !milliseconds.is_finite() || milliseconds.abs() > 8_640_000_000_000_000.0 {
        return Err(Exception::throw_range(ctx, "Invalid time value"));
    }
    let locale = resolved_locale(ctx, locales)?;
    let options: Value = serde_json::from_str(options_json)
        .map_err(|error| Exception::throw_type(ctx, &error.to_string()))?;
    let input = date_time_input(ctx, milliseconds, &options)?;
    let key = format!("{locales}\u{1}{options_json}");
    with_cached(
        &DATE_TIME_FORMATTERS,
        &key,
        || date_time_formatter(ctx, locale, &options),
        |formatter| {
            if parts {
                let mut collector = PartsCollector::default();
                formatter
                    .format(&input)
                    .write_to_parts(&mut collector)
                    .map_err(|_| Exception::throw_internal(ctx, "Failed to format date parts"))?;
                serde_json::to_string(&collector.parts)
                    .map_err(|error| Exception::throw_internal(ctx, &error.to_string()))
            } else {
                Ok(formatter.format(&input).to_string())
            }
        },
    )?
}

fn date_time_input(
    ctx: &Ctx<'_>,
    milliseconds: f64,
    options: &Value,
) -> rquickjs::Result<ZonedDateTime<Iso, TimeZoneInfo<AtTime>>> {
    #[allow(clippy::cast_possible_truncation)]
    let timestamp = Timestamp::from_millisecond(milliseconds as i64)
        .map_err(|error| Exception::throw_range(ctx, &error.to_string()))?;
    let time_zone_name = options
        .get("timeZone")
        .and_then(Value::as_str)
        .unwrap_or("UTC");
    let time_zone = jiff::tz::TimeZone::get(time_zone_name)
        .map_err(|error| Exception::throw_range(ctx, &error.to_string()))?;
    let local = timestamp.to_zoned(time_zone);
    let offset = UtcOffset::try_from_seconds(local.offset().seconds())
        .map_err(|_| Exception::throw_range(ctx, "Invalid time zone offset"))?;
    let date = Date::try_new_iso(
        i32::from(local.year()),
        local.month().cast_unsigned(),
        local.day().cast_unsigned(),
    )
    .map_err(|error| Exception::throw_range(ctx, &error.to_string()))?;
    let time = icu::time::Time::try_new(
        local.hour().cast_unsigned(),
        local.minute().cast_unsigned(),
        local.second().cast_unsigned(),
        u32::from(local.nanosecond().cast_unsigned()),
    )
    .map_err(|error| Exception::throw_range(ctx, &error.to_string()))?;
    let parser = icu::time::zone::iana::IanaParser::try_new_unstable(&*PROVIDER)
        .map_err(|error| Exception::throw_internal(ctx, &error.to_string()))?;
    let zone = parser
        .as_borrowed()
        .parse(time_zone_name)
        .with_offset(Some(offset))
        .with_zone_name_timestamp(ZoneNameTimestamp::from_epoch_seconds(timestamp.as_second()));
    Ok(ZonedDateTime { date, time, zone })
}

fn date_time_formatter(
    ctx: &Ctx<'_>,
    locale: Locale,
    options: &Value,
) -> rquickjs::Result<DateTimeFormatter<CompositeFieldSet>> {
    let field_set = field_set(ctx, options)?;
    let mut preferences = DateTimeFormatterPreferences::from(locale);
    if let Some(hour_cycle) = options
        .get("hourCycle")
        .and_then(Value::as_str)
        .and_then(parse_hour_cycle)
    {
        preferences.hour_cycle = Some(hour_cycle);
    } else if let Some(hour12) = options.get("hour12").and_then(Value::as_bool) {
        preferences.hour_cycle = Some(if hour12 {
            HourCycle::H12
        } else {
            HourCycle::H23
        });
    }
    DateTimeFormatter::try_new_unstable(&*PROVIDER, preferences, field_set)
        .map_err(|error| Exception::throw_range(ctx, &error.to_string()))
}

fn field_set(ctx: &Ctx<'_>, options: &Value) -> rquickjs::Result<CompositeFieldSet> {
    let date_style = options.get("dateStyle").and_then(Value::as_str);
    let time_style = options.get("timeStyle").and_then(Value::as_str);
    let has_date_option = ["weekday", "year", "month", "day"]
        .into_iter()
        .any(|key| options.get(key).is_some());
    let has_time_option = [
        "dayPeriod",
        "hour",
        "minute",
        "second",
        "fractionalSecondDigits",
    ]
    .into_iter()
    .any(|key| options.get(key).is_some());
    let has_date =
        date_style.is_some() || has_date_option || (!has_time_option && time_style.is_none());
    let has_time = time_style.is_some() || has_time_option;
    let mut builder = FieldSetBuilder::new();
    if options.get("year").and_then(Value::as_str) == Some("numeric") {
        builder.year_style = Some(YearStyle::Full);
    }
    builder.date_fields = has_date.then(|| date_fields(options, date_style));
    builder.time_precision = has_time.then(|| time_precision(options, time_style));
    builder.length = Some(length(date_style.or(time_style).or_else(|| {
        match options.get("month").and_then(Value::as_str) {
            Some("long") => Some("long"),
            Some("numeric" | "2-digit") => Some("short"),
            _ => None,
        }
    })));
    if options.get("timeZoneName").is_some() || matches!(time_style, Some("long" | "full")) {
        builder.zone_style = Some(match options.get("timeZoneName").and_then(Value::as_str) {
            Some("long") => ZoneStyle::SpecificLong,
            _ => ZoneStyle::SpecificShort,
        });
    }
    builder
        .build_composite()
        .map_err(|error| Exception::throw_range(ctx, &error.to_string()))
}

fn date_fields(options: &Value, style: Option<&str>) -> DateFields {
    if let Some(style) = style {
        return if style == "full" {
            DateFields::YMDE
        } else {
            DateFields::YMD
        };
    }
    let year = options.get("year").is_some();
    let month = options.get("month").is_some();
    let day = options.get("day").is_some();
    let weekday = options.get("weekday").is_some();
    match (year, month, day, weekday) {
        (true, true, true, true) => DateFields::YMDE,
        (false, true, true, true) => DateFields::MDE,
        (false, false, true, true) => DateFields::DE,
        (false, false, false, true) => DateFields::E,
        (true, true, true, false) => DateFields::YMD,
        (true, true, false, false) => DateFields::YM,
        (true, false, false, false) => DateFields::Y,
        (false, true, true, false) => DateFields::MD,
        (false, true, false, false) => DateFields::M,
        _ => DateFields::D,
    }
}

fn time_precision(options: &Value, style: Option<&str>) -> TimePrecision {
    if matches!(style, Some("long" | "full")) {
        return TimePrecision::Second;
    }
    if style == Some("short")
        || options.get("hour").is_some()
            && options.get("minute").is_none()
            && options.get("second").is_none()
    {
        return TimePrecision::Hour;
    }
    if options.get("second").is_none() && options.get("fractionalSecondDigits").is_none() {
        TimePrecision::Minute
    } else {
        TimePrecision::Second
    }
}

fn length(style: Option<&str>) -> Length {
    match style {
        Some("full" | "long") => Length::Long,
        Some("short") => Length::Short,
        _ => Length::Medium,
    }
}

fn parse_hour_cycle(value: &str) -> Option<HourCycle> {
    match value {
        "h11" => Some(HourCycle::H11),
        "h12" => Some(HourCycle::H12),
        "h23" | "h24" => Some(HourCycle::H23),
        _ => None,
    }
}

fn format_number(
    ctx: &Ctx<'_>,
    value: f64,
    locale: &Locale,
    options: &Value,
) -> rquickjs::Result<String> {
    Ok(format_number_parts_inner(ctx, value, locale, options)?
        .into_iter()
        .map(|(_, value)| value)
        .collect())
}

fn format_number_parts(
    ctx: &Ctx<'_>,
    value: f64,
    locale: &Locale,
    options: &Value,
) -> rquickjs::Result<String> {
    serde_json::to_string(&format_number_parts_inner(ctx, value, locale, options)?)
        .map_err(|error| Exception::throw_internal(ctx, &error.to_string()))
}

fn format_number_parts_inner(
    ctx: &Ctx<'_>,
    value: f64,
    locale: &Locale,
    options: &Value,
) -> rquickjs::Result<Vec<(String, String)>> {
    if value.is_nan() {
        return Ok(vec![("nan".to_owned(), "NaN".to_owned())]);
    }
    if value.is_infinite() {
        let mut parts = Vec::new();
        if value.is_sign_negative() {
            parts.push(("minusSign".to_owned(), "-".to_owned()));
        }
        parts.push(("infinity".to_owned(), "∞".to_owned()));
        return Ok(parts);
    }

    let style = options
        .get("style")
        .and_then(Value::as_str)
        .unwrap_or("decimal");
    let notation = options
        .get("notation")
        .and_then(Value::as_str)
        .unwrap_or("standard");
    let decimal = prepared_number(ctx, value, style, notation, options)?;

    match notation {
        "compact" => compact_parts(ctx, locale, &decimal, options),
        "scientific" | "engineering" => {
            scientific_parts(ctx, locale, &decimal, notation == "engineering")
        }
        _ => match style {
            "currency" => currency_parts(ctx, locale, &decimal, options),
            "percent" => percent_parts(ctx, locale, &decimal, options),
            "unit" => unit_parts(ctx, locale, &decimal, options),
            _ => decimal_parts(ctx, locale, &decimal, options),
        },
    }
}

fn prepared_number(
    ctx: &Ctx<'_>,
    value: f64,
    style: &str,
    notation: &str,
    options: &Value,
) -> rquickjs::Result<FixedDecimal> {
    let mut decimal = value
        .to_string()
        .parse::<FixedDecimal>()
        .map_err(|error| Exception::throw_range(ctx, &error.to_string()))?;
    if style == "percent" {
        decimal.multiply_pow10(2);
    }
    if notation == "compact" {
        decimal.apply_sign_display(sign_display(options));
        return Ok(decimal);
    }

    let (minimum_fraction, maximum_fraction) = fraction_digits(style, options);
    decimal.round_with_mode(
        -(i16::try_from(maximum_fraction).unwrap_or(i16::MAX)),
        rounding_mode(options),
    );
    decimal.pad_end(-(i16::try_from(minimum_fraction).unwrap_or(i16::MAX)));
    decimal.pad_start(
        i16::try_from(
            options
                .get("minimumIntegerDigits")
                .and_then(Value::as_u64)
                .unwrap_or(1),
        )
        .unwrap_or(1),
    );
    decimal.apply_sign_display(sign_display(options));
    Ok(decimal)
}

fn fraction_digits(style: &str, options: &Value) -> (u64, u64) {
    let currency_digits = options
        .get("currency")
        .and_then(Value::as_str)
        .map_or(2, currency_fraction_digits);
    let default_minimum = match style {
        "currency" => currency_digits,
        _ => 0,
    };
    let default_maximum = match style {
        "currency" => currency_digits,
        "percent" => 0,
        _ => 3,
    };
    let minimum = options
        .get("minimumFractionDigits")
        .and_then(Value::as_u64)
        .unwrap_or(default_minimum);
    let maximum = options
        .get("maximumFractionDigits")
        .and_then(Value::as_u64)
        .unwrap_or(default_maximum.max(minimum));
    (minimum.min(100), maximum.max(minimum).min(100))
}

// icu4x ships CLDR fraction data only behind doc-hidden provider internals;
// this table covers the common non-default currencies, the rest default to 2.
fn currency_fraction_digits(currency: &str) -> u64 {
    match currency {
        "BHD" | "IQD" | "JOD" | "KWD" | "LYD" | "OMR" | "TND" => 3,
        "CLP" | "ISK" | "JPY" | "KRW" | "PYG" | "RWF" | "UGX" | "VND" | "VUV" | "XAF" | "XOF"
        | "XPF" => 0,
        _ => 2,
    }
}

fn rounding_mode(options: &Value) -> SignedRoundingMode {
    match options.get("roundingMode").and_then(Value::as_str) {
        Some("ceil") => SignedRoundingMode::Ceil,
        Some("floor") => SignedRoundingMode::Floor,
        Some("expand") => SignedRoundingMode::Unsigned(UnsignedRoundingMode::Expand),
        Some("trunc") => SignedRoundingMode::Unsigned(UnsignedRoundingMode::Trunc),
        Some("halfCeil") => SignedRoundingMode::HalfCeil,
        Some("halfFloor") => SignedRoundingMode::HalfFloor,
        Some("halfTrunc") => SignedRoundingMode::Unsigned(UnsignedRoundingMode::HalfTrunc),
        Some("halfEven") => SignedRoundingMode::Unsigned(UnsignedRoundingMode::HalfEven),
        _ => SignedRoundingMode::Unsigned(UnsignedRoundingMode::HalfExpand),
    }
}

fn sign_display(options: &Value) -> SignDisplay {
    match options.get("signDisplay").and_then(Value::as_str) {
        Some("never") => SignDisplay::Never,
        Some("always") => SignDisplay::Always,
        Some("exceptZero") => SignDisplay::ExceptZero,
        Some("negative") => SignDisplay::Negative,
        _ => SignDisplay::Auto,
    }
}

fn grouping_strategy(options: &Value) -> GroupingStrategy {
    match options.get("useGrouping") {
        Some(Value::Bool(false)) => GroupingStrategy::Never,
        Some(Value::String(value)) if value == "never" => GroupingStrategy::Never,
        Some(Value::String(value)) if value == "always" => GroupingStrategy::Always,
        Some(Value::String(value)) if value == "min2" => GroupingStrategy::Min2,
        _ => GroupingStrategy::Auto,
    }
}

fn decimal_parts(
    ctx: &Ctx<'_>,
    locale: &Locale,
    decimal: &FixedDecimal,
    options: &Value,
) -> rquickjs::Result<Vec<(String, String)>> {
    with_decimal_formatter(ctx, locale, grouping_strategy(options), |formatter| {
        let mut collector = PartsCollector::default();
        formatter
            .format(decimal)
            .write_to_parts(&mut collector)
            .map_err(|_| Exception::throw_internal(ctx, "Failed to format number parts"))?;
        Ok(split_sign_marks(collector.parts))
    })?
}

/// Bidi marks around a sign are literals of their own.
fn split_sign_marks(parts: Vec<(String, String)>) -> Vec<(String, String)> {
    let mut result = Vec::with_capacity(parts.len());
    for (kind, value) in parts {
        if kind != "minusSign" && kind != "plusSign" {
            result.push((kind, value));
            continue;
        }
        let start = value.len() - value.trim_start_matches(is_bidi_mark).len();
        let sign = value[start..].trim_end_matches(is_bidi_mark);
        let end = start + sign.len();
        append_literal_parts(&mut result, &value[..start]);
        result.push((kind, sign.to_owned()));
        append_literal_parts(&mut result, &value[end..]);
    }
    result
}

fn is_bidi_mark(character: char) -> bool {
    matches!(character, '\u{200e}' | '\u{200f}' | '\u{61c}')
}

fn compact_parts(
    ctx: &Ctx<'_>,
    locale: &Locale,
    decimal: &FixedDecimal,
    options: &Value,
) -> rquickjs::Result<Vec<(String, String)>> {
    let formatter = if options.get("compactDisplay").and_then(Value::as_str) == Some("long") {
        CompactDecimalFormatter::try_new_long_unstable(
            &*PROVIDER,
            locale.clone().into(),
            CompactDecimalFormatterOptions::default(),
        )
    } else {
        CompactDecimalFormatter::try_new_short_unstable(
            &*PROVIDER,
            locale.clone().into(),
            CompactDecimalFormatterOptions::default(),
        )
    }
    .map_err(|error| Exception::throw_range(ctx, &error.to_string()))?;
    let mut collector = PartsCollector::default();
    formatter
        .format(decimal)
        .write_to_parts(&mut collector)
        .map_err(|_| Exception::throw_internal(ctx, "Failed to format compact number parts"))?;
    Ok(collector.parts)
}

fn currency_parts(
    ctx: &Ctx<'_>,
    locale: &Locale,
    decimal: &FixedDecimal,
    options: &Value,
) -> rquickjs::Result<Vec<(String, String)>> {
    let code = options
        .get("currency")
        .and_then(Value::as_str)
        .unwrap_or("XXX");
    let currency = CurrencyType::try_from_str(code)
        .map_err(|error| Exception::throw_range(ctx, &error.to_string()))?;
    let preferences = CurrencyFormatterPreferences::from(locale.clone());
    let mut formatter_options = CurrencyFormatterOptions::default();
    formatter_options.usage =
        if options.get("currencySign").and_then(Value::as_str) == Some("accounting") {
            CurrencyUsage::Accounting
        } else {
            CurrencyUsage::Standard
        };
    let display = options
        .get("currencyDisplay")
        .and_then(Value::as_str)
        .unwrap_or("symbol");
    let formatted = match display {
        "code" => CurrencyFormatter::try_new_code_unstable(
            &*PROVIDER,
            preferences,
            currency,
            formatter_options,
        ),
        "name" => CurrencyFormatter::try_new_name_unstable(&*PROVIDER, preferences, currency),
        "narrowSymbol" => CurrencyFormatter::try_new_symbol_narrow_unstable(
            &*PROVIDER,
            preferences,
            currency,
            formatter_options,
        ),
        _ => CurrencyFormatter::try_new_symbol_unstable(
            &*PROVIDER,
            preferences,
            currency,
            formatter_options,
        ),
    }
    .map_err(|error| Exception::throw_range(ctx, &error.to_string()))?
    .format_fixed_decimal(decimal)
    .to_string();
    affix_parts(ctx, locale, decimal, options, &formatted, "currency")
}

fn percent_parts(
    ctx: &Ctx<'_>,
    locale: &Locale,
    decimal: &FixedDecimal,
    options: &Value,
) -> rquickjs::Result<Vec<(String, String)>> {
    let preferences = PercentFormatterPreferences::from(locale.clone());
    let formatted = PercentFormatter::try_new_unstable(
        &*PROVIDER,
        preferences,
        PercentFormatterOptions::default(),
    )
    .map_err(|error| Exception::throw_range(ctx, &error.to_string()))?
    .format(decimal)
    .to_string();
    affix_parts(ctx, locale, decimal, options, &formatted, "percentSign")
}

fn unit_parts(
    ctx: &Ctx<'_>,
    locale: &Locale,
    decimal: &FixedDecimal,
    options: &Value,
) -> rquickjs::Result<Vec<(String, String)>> {
    let mut number_options = options.clone();
    number_options["style"] = Value::String("decimal".to_owned());
    let numeric_parts = decimal_parts(ctx, locale, decimal, &number_options)?;
    let unit = options.get("unit").and_then(Value::as_str).unwrap_or("");
    let display = options
        .get("unitDisplay")
        .and_then(Value::as_str)
        .unwrap_or("short");
    let rules =
        PluralRules::try_new_cardinal_unstable(&*PROVIDER, plural_locale(ctx, locale)?.into())
            .map_err(|error| Exception::throw_range(ctx, &error.to_string()))?;
    let plural = plural_category(rules.category_for(decimal));
    let pattern = units::pattern(ctx, locale, unit, display, plural)?;
    Ok(units::parts(&pattern, numeric_parts))
}

fn scientific_parts(
    ctx: &Ctx<'_>,
    locale: &Locale,
    decimal: &FixedDecimal,
    engineering: bool,
) -> rquickjs::Result<Vec<(String, String)>> {
    if decimal.absolute.is_zero() {
        return decimal_parts(
            ctx,
            locale,
            decimal,
            &serde_json::json!({"useGrouping": false}),
        );
    }
    let mut exponent = i32::from(decimal.absolute.nonzero_magnitude_start());
    if engineering {
        exponent -= exponent.rem_euclid(3);
    }
    let shift = i16::try_from(exponent).unwrap_or(0);
    let mut coefficient = decimal.clone();
    coefficient.multiply_pow10(-shift);
    coefficient.round_with_mode(
        -3,
        SignedRoundingMode::Unsigned(UnsignedRoundingMode::HalfExpand),
    );
    let number_options = serde_json::json!({
        "useGrouping": false,
        "minimumFractionDigits": 0,
        "maximumFractionDigits": 3,
    });
    let mut parts = decimal_parts(ctx, locale, &coefficient, &number_options)?;
    parts.push(("exponentSeparator".to_owned(), "E".to_owned()));
    if exponent < 0 {
        parts.push(("exponentMinusSign".to_owned(), "-".to_owned()));
    }
    parts.push((
        "exponentInteger".to_owned(),
        exponent.unsigned_abs().to_string(),
    ));
    Ok(parts)
}

/// Parts for a formatted number whose affixes carry the sign and one `token_kind` token.
fn affix_parts(
    ctx: &Ctx<'_>,
    locale: &Locale,
    decimal: &FixedDecimal,
    options: &Value,
    formatted: &str,
    token_kind: &str,
) -> rquickjs::Result<Vec<(String, String)>> {
    let mut number_options = options.clone();
    number_options["style"] = Value::String("decimal".to_owned());
    let sign = decimal_parts(ctx, locale, decimal, &number_options)?
        .into_iter()
        .find(|(kind, _)| kind == "minusSign" || kind == "plusSign");
    let number = decimal.clone().with_sign(Sign::None);
    let numeric_parts = decimal_parts(ctx, locale, &number, &number_options)?;
    Ok(split_affixes(
        formatted,
        numeric_parts,
        sign.as_ref(),
        token_kind,
    ))
}

fn split_affixes(
    formatted: &str,
    numeric_parts: Vec<(String, String)>,
    sign: Option<&(String, String)>,
    token_kind: &str,
) -> Vec<(String, String)> {
    let numeric_text: String = numeric_parts
        .iter()
        .map(|(_, value)| value.as_str())
        .collect();
    let span = formatted
        .find(&numeric_text)
        .filter(|_| !numeric_text.is_empty())
        .map(|start| (start, start + numeric_text.len()))
        .or_else(|| numeric_range(formatted));
    let Some((start, end)) = span else {
        return vec![("literal".to_owned(), formatted.to_owned())];
    };
    let parenthesised = formatted.starts_with('(') && formatted.ends_with(')');
    let mut parts = Vec::new();
    let affix = Affix {
        sign,
        token_kind,
        parenthesised,
    };
    affix.append(&mut parts, &formatted[..start], true);
    parts.extend(numeric_parts);
    affix.append(&mut parts, &formatted[end..], false);
    parts
}

struct Affix<'a> {
    sign: Option<&'a (String, String)>,
    token_kind: &'a str,
    parenthesised: bool,
}

impl Affix<'_> {
    /// Splits one affix into parentheses, the sign, whitespace literals and the token.
    fn append(&self, parts: &mut Vec<(String, String)>, affix: &str, prefix: bool) {
        let mut rest = affix;
        let mut opening = false;
        let mut closing = false;
        if prefix
            && self.parenthesised
            && let Some(remaining) = rest.strip_prefix('(')
        {
            opening = true;
            rest = remaining;
        }
        if !prefix
            && self.parenthesised
            && let Some(remaining) = rest.strip_suffix(')')
        {
            closing = true;
            rest = remaining;
        }
        let (outer_lead, inner, outer_trail) = split_literal_edges(rest);
        rest = inner;
        let mut leading_sign = None;
        let mut trailing_sign = None;
        if let Some((kind, value)) = self.sign {
            if let Some(remaining) = rest.strip_prefix(value.as_str()) {
                leading_sign = Some((kind, value));
                rest = remaining;
            } else if let Some(remaining) = rest.strip_suffix(value.as_str()) {
                trailing_sign = Some((kind, value));
                rest = remaining;
            }
        }
        let (inner_lead, token, inner_trail) = split_literal_edges(rest);
        if opening {
            append_literal_parts(parts, "(");
        }
        append_literal_parts(parts, outer_lead);
        if let Some((kind, value)) = leading_sign {
            parts.push((kind.clone(), value.clone()));
        }
        append_literal_parts(parts, inner_lead);
        if !token.is_empty() {
            parts.push((self.token_kind.to_owned(), token.to_owned()));
        }
        append_literal_parts(parts, inner_trail);
        if let Some((kind, value)) = trailing_sign {
            parts.push((kind.clone(), value.clone()));
        }
        append_literal_parts(parts, outer_trail);
        if closing {
            append_literal_parts(parts, ")");
        }
    }
}

/// Splits the whitespace and bidi marks off both ends of an affix segment.
fn split_literal_edges(value: &str) -> (&str, &str, &str) {
    let start = value.len() - value.trim_start_matches(is_affix_literal).len();
    let inner = value[start..].trim_end_matches(is_affix_literal);
    let end = start + inner.len();
    (&value[..start], inner, &value[end..])
}

fn is_affix_literal(character: char) -> bool {
    character.is_whitespace() || is_bidi_mark(character)
}

fn append_literal_parts(parts: &mut Vec<(String, String)>, value: &str) {
    if value.is_empty() {
        return;
    }
    if let Some((kind, existing)) = parts.last_mut()
        && kind == "literal"
    {
        existing.push_str(value);
    } else {
        parts.push(("literal".to_owned(), value.to_owned()));
    }
}

fn numeric_range(value: &str) -> Option<(usize, usize)> {
    let start = value
        .char_indices()
        .find(|(_, character)| character.is_numeric())
        .map(|(index, _)| index)?;
    let end = value
        .char_indices()
        .rev()
        .find(|(_, character)| character.is_numeric())
        .map(|(index, character)| index + character.len_utf8())?;
    Some((start, end))
}

#[derive(Default)]
struct PartsCollector {
    parts: Vec<(String, String)>,
    stack: Vec<Part>,
}

impl fmt::Write for PartsCollector {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        let kind = self
            .stack
            .iter()
            .rev()
            .find(|part| part.category == "datetime")
            .or_else(|| self.stack.last())
            .map_or("literal", |part| part.value);
        if let Some((last_kind, existing)) = self.parts.last_mut()
            && last_kind == kind
        {
            existing.push_str(value);
        } else if !value.is_empty() {
            self.parts.push((kind.to_owned(), value.to_owned()));
        }
        Ok(())
    }
}

impl PartsWrite for PartsCollector {
    type SubPartsWrite = Self;

    fn with_part(
        &mut self,
        part: Part,
        mut write: impl FnMut(&mut Self) -> fmt::Result,
    ) -> fmt::Result {
        self.stack.push(part);
        let result = write(self);
        self.stack.pop();
        result
    }
}
