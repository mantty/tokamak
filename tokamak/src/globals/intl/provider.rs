//! Trimmed ICU4X data provider.
//!
//! Bundling all ~800 CLDR locales costs ~12 MB. Cloudflare Workers (workerd/V8)
//! ships display data for only a curated set and falls unsupported locales back
//! to the runtime default (`en-US`). We match that: `icu_data.postcard` holds the
//! same set workerd localizes, and [`resolve`] sends anything outside it to
//! `en-US`, so an app that works on Cloudflare formats identically here.
//!
//! The `expect`s below load data embedded at compile time; a failure means the
//! bundled blob is corrupt, which the `Intl` differential tests catch.
//!
//! To regenerate `icu_data.postcard` (e.g. after bumping `icu`): build the
//! runtime library, then run `icu4x-datagen` (installed with `--features unstable`)
//! with `--markers-for-bin` pointing at it to list the markers it references,
//! remove the four listed in `scripts/icu-excluded-markers.txt`, and run again
//! with that list as `--markers`, `--deduplication maximal`, `--format blob` and
//! `--locales` set to the tags in `scripts/icu-locales.txt` — the locales
//! workerd localizes datetime names for, plus every `en-*` variant — then
//! `zstd -19` the output to `icu_data.postcard.zst`. Keep `SUPPORTED` below in
//! sync with those tags' base languages.
#![allow(clippy::expect_used)]

use std::sync::LazyLock;

use icu::locale::fallback::LocaleFallbacker;
use icu::locale::{Locale, locale};
use icu_provider::buf::{AsDeserializingBufferProvider, DeserializingBufferProvider};
use icu_provider_adapters::fallback::LocaleFallbackProvider;
use icu_provider_blob::BlobDataProvider;

// The blob is stored zstd-compressed (~1.4 MB vs ~7 MB) so it costs little in the
// binary, and is inflated once into a process-wide provider on first `Intl` use.
// `zstd` is already linked for the compression codecs, so this adds no dependency.
static DATA_ZST: &[u8] = include_bytes!("icu_data.postcard.zst");

static BLOB: LazyLock<BlobDataProvider> = LazyLock::new(|| {
    let blob = zstd::decode_all(DATA_ZST).expect("valid zstd ICU data");
    BlobDataProvider::try_new_from_blob(blob.into_boxed_slice()).expect("valid ICU data blob")
});

/// Locale fallbacker built from the bundled data, shared by the formatter
/// provider and the units lookup.
pub(super) static FALLBACKER: LazyLock<LocaleFallbacker> = LazyLock::new(|| {
    LocaleFallbacker::try_new_unstable(&BLOB.as_deserializing()).expect("locale fallback data")
});

/// Data provider used by every `Intl` formatter: the trimmed blob with locale
/// fallback (so `de-AT` resolves to `de`, etc.).
pub(super) static PROVIDER: LazyLock<
    LocaleFallbackProvider<DeserializingBufferProvider<'static, BlobDataProvider>>,
> = LazyLock::new(|| LocaleFallbackProvider::new(BLOB.as_deserializing(), FALLBACKER.clone()));

/// Base language subtags for which the bundled data has real CLDR data. Anything
/// else is not localized by workerd either, so it falls back to `en-US`.
static SUPPORTED: &[&str] = &[
    "af", "ak", "am", "ar", "az", "bg", "bn", "bs", "ca", "ckb", "cs", "da", "de", "ee", "el",
    "en", "es", "et", "fa", "fi", "fil", "fr", "gu", "ha", "he", "hi", "hr", "hu", "id", "it",
    "ja", "kk", "kn", "ko", "kok", "lt", "lv", "ml", "mo", "mr", "ms", "nb", "ne", "nl", "no",
    "oc", "om", "pa", "pl", "ps", "pt", "ro", "ru", "sd", "sh", "sk", "sl", "so", "sq", "sr", "st",
    "sv", "sw", "ta", "te", "th", "ti", "tl", "tr", "uk", "ur", "uz", "vi", "yo", "zh",
];

/// Returns the locale unchanged when its language has bundled data, otherwise
/// `en-US` — matching workerd's fallback for locales it does not localize.
pub(super) fn resolve(locale: Locale) -> Locale {
    if SUPPORTED.contains(&locale.id.language.as_str()) {
        locale
    } else {
        locale!("en-US")
    }
}
