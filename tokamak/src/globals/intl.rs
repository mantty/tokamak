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
use icu::collator::options::{
    AlternateHandling, CaseLevel, CollatorOptions, MaxVariable, Strength,
};
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
use rquickjs::{Ctx, Exception};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use writeable::{Part, PartsWrite, Writeable};

super::host_functions! {
    pub(super),
    "intlCanonicalLocales" => canonical_locales,
    "intlDateTime" => date_time,
    "intlDateTimeParts" => date_time_parts,
    "intlNumber" => number,
    "intlNumberParts" => number_parts,
    "intlPlural" => plural,
    "intlPluralRange" => plural_range,
    "intlPluralCategories" => plural_categories,
    "intlList" => list,
    "intlListParts" => list_parts,
    "intlRelative" => relative,
    "intlRelativeParts" => relative_parts,
    "intlCollatorCompare" => collator_compare,
    "intlSegment" => segment,
    "intlDisplayName" => display_name,
    "intlLocaleInfo" => locale_info,
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
            .map_err(range_error(ctx))
        },
        apply,
    )
}

/// Maps an error onto a JS `RangeError` carrying its message.
fn range_error<E: fmt::Display>(ctx: &Ctx<'_>) -> impl FnOnce(E) -> rquickjs::Error {
    move |error| Exception::throw_range(ctx, &error.to_string())
}

/// Maps an error onto a JS `InternalError` carrying its message.
fn internal_error<E: fmt::Display>(ctx: &Ctx<'_>) -> impl FnOnce(E) -> rquickjs::Error {
    move |error| Exception::throw_internal(ctx, &error.to_string())
}

/// Parses JSON sent from JS; malformed input is a `TypeError`.
fn from_json<T: DeserializeOwned>(ctx: &Ctx<'_>, input: &str) -> rquickjs::Result<T> {
    serde_json::from_str(input).map_err(|error| Exception::throw_type(ctx, &error.to_string()))
}

fn to_json<T: Serialize + ?Sized>(ctx: &Ctx<'_>, value: &T) -> rquickjs::Result<String> {
    serde_json::to_string(value).map_err(internal_error(ctx))
}

fn canonical_locales(ctx: Ctx<'_>, input: String) -> rquickjs::Result<String> {
    let values: Vec<String> = from_json(&ctx, &input)?;
    let canonicalizer =
        LocaleCanonicalizer::try_new_extended_unstable(&*PROVIDER).map_err(internal_error(&ctx))?;
    let mut output = Vec::with_capacity(values.len());
    for value in values {
        let mut locale = value.parse::<Locale>().map_err(range_error(&ctx))?;
        canonicalizer.canonicalize(&mut locale);
        let value = locale.to_string();
        if !output.contains(&value) {
            output.push(value);
        }
    }
    to_json(&ctx, &output)
}

fn date_time(
    ctx: Ctx<'_>,
    milliseconds: f64,
    locales: String,
    options: String,
) -> rquickjs::Result<String> {
    format_date_time(&ctx, milliseconds, &locales, &options, false)
}

fn date_time_parts(
    ctx: Ctx<'_>,
    milliseconds: f64,
    locales: String,
    options: String,
) -> rquickjs::Result<String> {
    format_date_time(&ctx, milliseconds, &locales, &options, true)
}

fn number(ctx: Ctx<'_>, value: f64, locales: String, options: String) -> rquickjs::Result<String> {
    let parts = format_number_parts(&ctx, value, &locales, &options)?;
    Ok(parts.into_iter().map(|(_, value)| value).collect())
}

fn number_parts(
    ctx: Ctx<'_>,
    value: f64,
    locales: String,
    options: String,
) -> rquickjs::Result<String> {
    to_json(&ctx, &format_number_parts(&ctx, value, &locales, &options)?)
}

fn plural(ctx: Ctx<'_>, value: f64, locales: String, kind: String) -> rquickjs::Result<String> {
    if !value.is_finite() {
        return Ok("other".to_owned());
    }
    let rules = plural_rules(&ctx, &locales, &kind)?;
    let decimal = parse_decimal(&ctx, value)?;
    Ok(plural_category(rules.category_for(&decimal)).to_owned())
}

fn plural_range(
    ctx: Ctx<'_>,
    start: f64,
    end: f64,
    locales: String,
    kind: String,
) -> rquickjs::Result<String> {
    if !start.is_finite() || !end.is_finite() {
        return Ok("other".to_owned());
    }
    let locale = plural_locale(&ctx, &resolved_locale(&ctx, &locales)?)?;
    let rules = PluralRulesWithRanges::try_new_unstable(
        &*PROVIDER,
        locale.into(),
        plural_rules_options(&kind),
    )
    .map_err(range_error(&ctx))?;
    let start = parse_decimal(&ctx, start)?;
    let end = parse_decimal(&ctx, end)?;
    Ok(plural_category(rules.category_for_range(&start, &end)).to_owned())
}

fn plural_categories(ctx: Ctx<'_>, locales: String, kind: String) -> rquickjs::Result<String> {
    let rules = plural_rules(&ctx, &locales, &kind)?;
    let values = rules.categories().map(plural_category).collect::<Vec<_>>();
    to_json(&ctx, &values)
}

fn list(
    ctx: Ctx<'_>,
    values: String,
    locales: String,
    options: String,
) -> rquickjs::Result<String> {
    let values: Vec<String> = from_json(&ctx, &values)?;
    let formatter = list_formatter(&ctx, &locales, &options)?;
    Ok(formatter.format_to_string(values.iter().map(String::as_str)))
}

fn list_parts(
    ctx: Ctx<'_>,
    values: String,
    locales: String,
    options: String,
) -> rquickjs::Result<String> {
    let values: Vec<String> = from_json(&ctx, &values)?;
    let formatter = list_formatter(&ctx, &locales, &options)?;
    let formatted_list = formatter.format(values.iter().map(String::as_str));
    let parts = collect_parts(&ctx, &formatted_list, "Failed to format list parts")?;
    to_json(&ctx, &parts)
}

fn relative(
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
    Ok((locale, from_json(ctx, options)?))
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
    .map_err(range_error(ctx))?;
    Ok(formatter.format(parse_decimal(ctx, value)?).to_string())
}

fn relative_parts(
    ctx: Ctx<'_>,
    value: f64,
    unit: String,
    locales: String,
    options: String,
) -> rquickjs::Result<String> {
    let (locale, options) = relative_inputs(&ctx, value, &locales, &options)?;
    let formatted = format_relative(&ctx, value, &unit, &locale, &options)?;
    let absolute = parse_decimal(&ctx, value.abs())?;
    let number = with_decimal_formatter(&ctx, &locale, GroupingStrategy::Auto, |formatter| {
        formatter.format_to_string(&absolute)
    })?;
    let Some(index) = formatted.find(&number) else {
        return to_json(
            &ctx,
            &[serde_json::json!({ "type": "literal", "value": formatted })],
        );
    };
    let prefix = &formatted[..index];
    let suffix = &formatted[index + number.len()..];
    let kind = if value.fract() == 0.0 {
        "integer"
    } else {
        "fraction"
    };
    let mut parts = Vec::new();
    if !prefix.is_empty() {
        parts.push(serde_json::json!({ "type": "literal", "value": prefix }));
    }
    parts.push(serde_json::json!({ "type": kind, "value": number, "unit": unit }));
    if !suffix.is_empty() {
        parts.push(serde_json::json!({ "type": "literal", "value": suffix }));
    }
    to_json(&ctx, &parts)
}

fn collator_compare(
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
    let options: Value = from_json(ctx, options)?;
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
    let sensitivity = options.get("sensitivity").and_then(Value::as_str);
    let mut collator_options = CollatorOptions::default();
    collator_options.strength = match sensitivity {
        Some("base" | "case") => Some(Strength::Primary),
        Some("accent") => Some(Strength::Secondary),
        Some("variant") => Some(Strength::Tertiary),
        _ => None,
    };
    if sensitivity == Some("case") {
        collator_options.case_level = Some(CaseLevel::On);
    }
    if options.get("ignorePunctuation").and_then(Value::as_bool) == Some(true) {
        collator_options.alternate_handling = Some(AlternateHandling::Shifted);
        collator_options.max_variable = Some(MaxVariable::Punctuation);
    }
    Collator::try_new_unstable(&*PROVIDER, prefs, collator_options).map_err(range_error(ctx))
}

fn segment(
    ctx: Ctx<'_>,
    text: String,
    locales: String,
    granularity: String,
) -> rquickjs::Result<String> {
    let _locale = first_locale(&ctx, &locales)?;
    let (mut boundaries, word_types): (Vec<usize>, _) = match granularity.as_str() {
        "grapheme" => {
            let segmenter = GraphemeClusterSegmenter::try_new_unstable(&*PROVIDER)
                .map_err(range_error(&ctx))?;
            (segmenter.as_borrowed().segment_str(&text).collect(), None)
        }
        "sentence" => {
            let segmenter =
                SentenceSegmenter::try_new_unstable(&*PROVIDER, SentenceBreakOptions::default())
                    .map_err(range_error(&ctx))?;
            (segmenter.as_borrowed().segment_str(&text).collect(), None)
        }
        "word" => {
            let segmenter = WordSegmenter::try_new_for_non_complex_scripts_unstable(
                &*PROVIDER,
                WordBreakOptions::default(),
            )
            .map_err(range_error(&ctx))?;
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
    to_json(&ctx, &values)
}

fn display_name(
    ctx: Ctx<'_>,
    code: String,
    locales: String,
    options: String,
) -> rquickjs::Result<String> {
    let locale = resolved_locale(&ctx, &locales)?;
    let options: Value = from_json(&ctx, &options)?;
    let kind = options
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("language");
    let short = options.get("style").and_then(Value::as_str) == Some("short");
    let prefs = DisplayNamesPreferences::from(locale.clone());
    let value = match kind {
        "language" => {
            let id = code.parse::<Language>().map_err(range_error(&ctx))?.into();
            let name_options = LanguageIdentifierDisplayNameOptions::default();
            if short {
                LanguageIdentifierDisplayName::try_new_short_light_unstable(
                    &*PROVIDER,
                    prefs,
                    id,
                    name_options,
                )
            } else {
                LanguageIdentifierDisplayName::try_new_long_light_unstable(
                    &*PROVIDER,
                    prefs,
                    id,
                    name_options,
                )
            }
            .map_err(range_error(&ctx))?
            .as_borrowed()
            .to_string()
        }
        "region" => {
            let region = code.parse::<Region>().map_err(range_error(&ctx))?;
            if short {
                RegionDisplayName::try_new_short_light_unstable(&*PROVIDER, prefs, region)
            } else {
                RegionDisplayName::try_new_light_unstable(&*PROVIDER, prefs, region)
            }
            .map_err(range_error(&ctx))?
            .to_string()
        }
        "script" => {
            let script = code.parse::<Script>().map_err(range_error(&ctx))?;
            ScriptDisplayName::try_new_light_unstable(&*PROVIDER, prefs, script)
                .map_err(range_error(&ctx))?
                .to_string()
        }
        "calendar" | "currency" | "dateTimeField" if locale.id.language.as_str() == "en" => {
            english_display_name(kind, &code).to_owned()
        }
        _ => code,
    };
    Ok(value)
}

// icu4x 2.3 exposes no display-name API for calendars, currencies, or
// date-time fields; en keeps a small table, other locales return the code.
fn english_display_name<'a>(kind: &str, code: &'a str) -> &'a str {
    match (kind, code) {
        ("calendar", "gregory") => "Gregorian Calendar",
        ("calendar", "buddhist") => "Buddhist Calendar",
        ("calendar", "japanese") => "Japanese Calendar",
        ("calendar", "islamic") => "Islamic Calendar",
        ("currency", "GBP") => "British Pound",
        ("currency", "USD") => "US Dollar",
        ("currency", "EUR") => "Euro",
        ("currency", "JPY") => "Japanese Yen",
        ("dateTimeField", "weekOfYear" | "weekOfMonth") => "week",
        ("dateTimeField", "dayOfWeek") => "day of the week",
        ("dateTimeField", "dayperiod" | "dayPeriod") => "AM/PM",
        ("dateTimeField", "zone") => "time zone",
        _ => code,
    }
}

fn locale_info(ctx: Ctx<'_>, tag: String, _options: String) -> rquickjs::Result<String> {
    let mut locale = tag.parse::<Locale>().map_err(range_error(&ctx))?;
    LocaleCanonicalizer::try_new_extended_unstable(&*PROVIDER)
        .map_err(internal_error(&ctx))?
        .canonicalize(&mut locale);
    let mut maximum = locale.clone();
    let expander =
        LocaleExpander::try_new_extended_unstable(&*PROVIDER).map_err(internal_error(&ctx))?;
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
    to_json(&ctx, &value)
}

fn first_locale(ctx: &Ctx<'_>, locales: &str) -> rquickjs::Result<String> {
    let values: Vec<String> = from_json(ctx, locales)?;
    Ok(values
        .into_iter()
        .next()
        .unwrap_or_else(|| "en-US".to_owned()))
}

// Parses the first requested locale and maps it onto the bundled data set,
// falling unsupported locales back to `en-US` exactly as workerd does.
fn resolved_locale(ctx: &Ctx<'_>, locales: &str) -> rquickjs::Result<Locale> {
    let locale = first_locale(ctx, locales)?
        .parse::<Locale>()
        .map_err(range_error(ctx))?;
    Ok(resolve(locale))
}

fn parse_decimal(ctx: &Ctx<'_>, value: f64) -> rquickjs::Result<FixedDecimal> {
    value
        .to_string()
        .parse::<FixedDecimal>()
        .map_err(range_error(ctx))
}

fn plural_locale(ctx: &Ctx<'_>, locale: &Locale) -> rquickjs::Result<Locale> {
    // Supplemental plural data uses subtag lookup, not display-data parent
    // overrides (which would route sr-Latn to root instead of Serbian).
    let name = locale.id.to_string();
    let mut candidate = name.as_str();
    loop {
        if candidate != "und" && PLURAL_LOCALES.binary_search(&candidate).is_ok() {
            return candidate.parse::<Locale>().map_err(internal_error(ctx));
        }
        let Some((parent, _)) = candidate.rsplit_once('-') else {
            return Ok(icu::locale::locale!("en"));
        };
        candidate = parent;
    }
}

fn plural_rules(ctx: &Ctx<'_>, locales: &str, kind: &str) -> rquickjs::Result<PluralRules> {
    let locale = plural_locale(ctx, &resolved_locale(ctx, locales)?)?;
    PluralRules::try_new_unstable(&*PROVIDER, locale.into(), plural_rules_options(kind))
        .map_err(range_error(ctx))
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

fn list_formatter(ctx: &Ctx<'_>, locales: &str, options: &str) -> rquickjs::Result<ListFormatter> {
    let locale = resolved_locale(ctx, locales)?;
    let options: Value = from_json(ctx, options)?;
    let length = match options.get("style").and_then(Value::as_str) {
        Some("narrow") => ListLength::Narrow,
        Some("short") => ListLength::Short,
        _ => ListLength::Wide,
    };
    let formatter_options = ListFormatterOptions::default().with_length(length);
    let preferences = locale.into();
    match options.get("type").and_then(Value::as_str) {
        Some("disjunction") => {
            ListFormatter::try_new_or_unstable(&*PROVIDER, preferences, formatter_options)
        }
        Some("unit") => {
            ListFormatter::try_new_unit_unstable(&*PROVIDER, preferences, formatter_options)
        }
        _ => ListFormatter::try_new_and_unstable(&*PROVIDER, preferences, formatter_options),
    }
    .map_err(range_error(ctx))
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
    let options: Value = from_json(ctx, options_json)?;
    let input = date_time_input(ctx, milliseconds, &options)?;
    let key = format!("{locales}\u{1}{options_json}");
    with_cached(
        &DATE_TIME_FORMATTERS,
        &key,
        || date_time_formatter(ctx, locale, &options),
        |formatter| {
            let formatted = formatter.format(&input);
            if parts {
                let parts = collect_parts(ctx, &formatted, "Failed to format date parts")?;
                to_json(ctx, &parts)
            } else {
                Ok(formatted.to_string())
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
    let timestamp = Timestamp::from_millisecond(milliseconds as i64).map_err(range_error(ctx))?;
    let time_zone_name = options
        .get("timeZone")
        .and_then(Value::as_str)
        .unwrap_or("UTC");
    let time_zone = jiff::tz::TimeZone::get(time_zone_name).map_err(range_error(ctx))?;
    let local = timestamp.to_zoned(time_zone);
    let offset = UtcOffset::try_from_seconds(local.offset().seconds())
        .map_err(|_| Exception::throw_range(ctx, "Invalid time zone offset"))?;
    let date = Date::try_new_iso(
        i32::from(local.year()),
        local.month().cast_unsigned(),
        local.day().cast_unsigned(),
    )
    .map_err(range_error(ctx))?;
    let time = icu::time::Time::try_new(
        local.hour().cast_unsigned(),
        local.minute().cast_unsigned(),
        local.second().cast_unsigned(),
        u32::from(local.nanosecond().cast_unsigned()),
    )
    .map_err(range_error(ctx))?;
    let parser = icu::time::zone::iana::IanaParser::try_new_unstable(&*PROVIDER)
        .map_err(internal_error(ctx))?;
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
        .map_err(range_error(ctx))
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
    builder.build_composite().map_err(range_error(ctx))
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

fn format_number_parts(
    ctx: &Ctx<'_>,
    value: f64,
    locales: &str,
    options: &str,
) -> rquickjs::Result<Vec<(String, String)>> {
    let locale = resolved_locale(ctx, locales)?;
    let options: Value = from_json(ctx, options)?;
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
    let decimal = prepared_number(ctx, value, style, notation, &options)?;

    match notation {
        "compact" => compact_parts(ctx, &locale, &decimal, &options),
        "scientific" | "engineering" => {
            scientific_parts(ctx, &locale, &decimal, notation == "engineering")
        }
        _ => match style {
            "currency" => currency_parts(ctx, &locale, &decimal, &options),
            "percent" => percent_parts(ctx, &locale, &decimal, &options),
            "unit" => unit_parts(ctx, &locale, &decimal, &options),
            _ => decimal_parts(ctx, &locale, &decimal, grouping_strategy(&options)),
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
    let mut decimal = parse_decimal(ctx, value)?;
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
    grouping: GroupingStrategy,
) -> rquickjs::Result<Vec<(String, String)>> {
    with_decimal_formatter(ctx, locale, grouping, |formatter| {
        let formatted = formatter.format(decimal);
        let parts = collect_parts(ctx, &formatted, "Failed to format number parts")?;
        Ok(split_sign_marks(parts))
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
        let (lead, sign, trail) = split_edges(&value, is_bidi_mark);
        append_literal_parts(&mut result, lead);
        result.push((kind, sign.to_owned()));
        append_literal_parts(&mut result, trail);
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
    let preferences = locale.clone().into();
    let formatter_options = CompactDecimalFormatterOptions::default();
    let formatter = if options.get("compactDisplay").and_then(Value::as_str) == Some("long") {
        CompactDecimalFormatter::try_new_long_unstable(&*PROVIDER, preferences, formatter_options)
    } else {
        CompactDecimalFormatter::try_new_short_unstable(&*PROVIDER, preferences, formatter_options)
    }
    .map_err(range_error(ctx))?;
    collect_parts(
        ctx,
        &formatter.format(decimal),
        "Failed to format compact number parts",
    )
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
    let currency = CurrencyType::try_from_str(code).map_err(range_error(ctx))?;
    let preferences = CurrencyFormatterPreferences::from(locale.clone());
    let mut formatter_options = CurrencyFormatterOptions::default();
    formatter_options.usage =
        if options.get("currencySign").and_then(Value::as_str) == Some("accounting") {
            CurrencyUsage::Accounting
        } else {
            CurrencyUsage::Standard
        };
    let formatted = match options.get("currencyDisplay").and_then(Value::as_str) {
        Some("code") => CurrencyFormatter::try_new_code_unstable(
            &*PROVIDER,
            preferences,
            currency,
            formatter_options,
        ),
        Some("name") => CurrencyFormatter::try_new_name_unstable(&*PROVIDER, preferences, currency),
        Some("narrowSymbol") => CurrencyFormatter::try_new_symbol_narrow_unstable(
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
    .map_err(range_error(ctx))?
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
    .map_err(range_error(ctx))?
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
    let numeric_parts = decimal_parts(ctx, locale, decimal, grouping_strategy(options))?;
    let unit = options.get("unit").and_then(Value::as_str).unwrap_or("");
    let display = options
        .get("unitDisplay")
        .and_then(Value::as_str)
        .unwrap_or("short");
    let rules =
        PluralRules::try_new_cardinal_unstable(&*PROVIDER, plural_locale(ctx, locale)?.into())
            .map_err(range_error(ctx))?;
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
        return decimal_parts(ctx, locale, decimal, GroupingStrategy::Never);
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
    let mut parts = decimal_parts(ctx, locale, &coefficient, GroupingStrategy::Never)?;
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
    let grouping = grouping_strategy(options);
    let sign = decimal_parts(ctx, locale, decimal, grouping)?
        .into_iter()
        .find(|(kind, _)| kind == "minusSign" || kind == "plusSign");
    let number = decimal.clone().with_sign(Sign::None);
    let numeric_parts = decimal_parts(ctx, locale, &number, grouping)?;
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
        let (outer_lead, inner, outer_trail) = split_edges(rest, is_affix_literal);
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
        let (inner_lead, token, inner_trail) = split_edges(rest, is_affix_literal);
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

/// Splits the characters matching `is_edge` off both ends of `value`.
fn split_edges(value: &str, is_edge: fn(char) -> bool) -> (&str, &str, &str) {
    let start = value.len() - value.trim_start_matches(is_edge).len();
    let inner = value[start..].trim_end_matches(is_edge);
    let end = start + inner.len();
    (&value[..start], inner, &value[end..])
}

/// Whitespace and bidi marks at the edges of an affix segment are literals.
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

/// The typed parts of `formatted`; `failure` is the error message if writing fails.
fn collect_parts(
    ctx: &Ctx<'_>,
    formatted: &impl Writeable,
    failure: &str,
) -> rquickjs::Result<Vec<(String, String)>> {
    let mut collector = PartsCollector::default();
    formatted
        .write_to_parts(&mut collector)
        .map_err(|_| Exception::throw_internal(ctx, failure))?;
    Ok(collector.parts)
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
