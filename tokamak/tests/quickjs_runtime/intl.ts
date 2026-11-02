import { member } from "./outcome.ts";

function outcome<T>(callback: () => T) {
  try { return callback(); }
  catch (error) { return { error: member(error, "name") }; }
}

// Numeric-date zero padding and the AM/PM separator (U+202F vs space) differ
// between icu4x and V8/ICU; neither is a parity requirement, so date strings are
// compared with both normalized away (symmetric across runtimes).
function normalizeDate(value: string) {
  return value.replace(/ /g, " ").replace(/\b0(\d)/g, "$1");
}

export function intlContracts() {
  const result: Record<string, unknown> = {};
  const date = new Date("2024-07-01T12:34:56Z");
  result.dateMethods = ["en-GB", "de-DE", "en-US"].map(locale => ({
    date: normalizeDate(date.toLocaleDateString(locale, { timeZone: "Europe/London" })),
    time: normalizeDate(date.toLocaleTimeString(locale, { timeZone: "Europe/London" })),
    both: normalizeDate(date.toLocaleString(locale, { timeZone: "Europe/London" })),
  }));
  result.invalidDate = new Date(NaN).toLocaleString("en-GB");
  result.numberMethods = [1234.5, -42, 0, NaN, Infinity].map(value => value.toLocaleString("de-DE"));
  result.arrayNumbers = [1234.5, 6.7].toLocaleString("de-DE");
  result.typedNumbers = new Float64Array([1234.5, 6.7]).toLocaleString("de-DE");
  result.currencyDisplays = ([{ currencySign: "accounting" }, { currencyDisplay: "name" }, { currencyDisplay: "narrowSymbol" }] satisfies Intl.NumberFormatOptions[]).map(options => {
    const formatter = new Intl.NumberFormat("en-US", { style: "currency", currency: "USD", ...options });
    return [-1234.5, 1234.5].map(value => formatter.format(value));
  });
  const partStrings = (parts: Intl.NumberFormatPart[]) => parts.map(part => `${part.type}:${part.value}`);
  result.currencyParts = Object.fromEntries(["en-US", "de-DE", "sv-SE", "ar-EG", "nl-NL"].flatMap(locale =>
    // ICU4X places an explicit plus sign before the currency in nl-NL where V8 places it after the currency.
    ([{}, { currencyDisplay: "narrowSymbol" }, { currencyDisplay: "name" }, { currencyDisplay: "code" }, { currencySign: "accounting" }, { signDisplay: "always" }] satisfies Intl.NumberFormatOptions[]).filter(options => !(locale === "nl-NL" && options.signDisplay)).flatMap(options =>
      [-1234.5, 1234.5].map(value => [
        `${locale} ${JSON.stringify(options)} ${value}`,
        partStrings(new Intl.NumberFormat(locale, { style: "currency", currency: "USD", ...options }).formatToParts(value)),
      ]))));
  // ICU4X formats the ar-EG percent sign as "%" where V8 uses "٪", so that locale is left out.
  result.percentParts = Object.fromEntries(["en-US", "de-DE", "sv-SE"].map(locale =>
    [locale, partStrings(new Intl.NumberFormat(locale, { style: "percent" }).formatToParts(-0.125))]));
  // @ts-expect-error Array toLocaleString passes on two arguments only.
  result.arrayLocaleArguments = [{ toLocaleString(...args: unknown[]) { return JSON.stringify(args); } }].toLocaleString("de-DE", { useGrouping: false }, "ignored");
  // @ts-expect-error Locale lists may be any array-like or iterable.
  result.localeLists = [{ 0: "en-gb", 2: "de", length: 3 }, new Intl.Locale("en-GB"), new Set(["de"])].map(locales => outcome(() => Intl.getCanonicalLocales(locales)));
  // @ts-expect-error The options are not dictionaries.
  result.optionPrimitives = [null, 3, true, "x"].map(options => outcome(() => { new Intl.DateTimeFormat("en", options); return true; }));
  // @ts-expect-error Strings and BigInts are not dates.
  result.dateNumbers = ["0", 0, 1n].map(value => outcome(() => new Intl.DateTimeFormat("en-GB", { dateStyle: "short", timeZone: "UTC" }).format(value)));
  result.collation = ["a", "ä", "2"].map(value => Math.sign(value.localeCompare("10", "de", { numeric: true })));
  result.timeZoneNames = Object.fromEntries([["en-US", "America/Los_Angeles"], ["de", "Europe/Berlin"], ["ja", "Asia/Tokyo"], ["en-GB", "Australia/Sydney"]].flatMap(([locale, timeZone]) =>
    (["short", "long"] as const).map(timeZoneName => [`${locale} ${timeZone} ${timeZoneName}`, outcome(() => new Intl.DateTimeFormat(locale, { timeZone, timeZoneName, hour: "numeric" }).formatToParts(date).find(part => part.type === "timeZoneName")?.value)])));
  result.ownProperties = [new Intl.DateTimeFormat("en"), new Intl.NumberFormat("en"), new Intl.PluralRules("en"), new Intl.Locale("en"), new Intl.ListFormat("en"), new Intl.RelativeTimeFormat("en"), new Intl.Collator("en"), new Intl.Segmenter("en"), new Intl.DisplayNames("en", { type: "region" })]
    .map(value => Reflect.ownKeys(value).map(String));
  result.collatorPrototype = Object.getOwnPropertyNames(Intl.Collator.prototype).sort();
  result.segmentData = (["grapheme", "word", "sentence"] as const).map(granularity => {
    const text = "Hi 🙂 there. é́x! Done";
    const segments = new Intl.Segmenter("en", { granularity }).segment(text);
    const values = [...segments];
    return {
      keys: Object.keys(values[0]),
      values: values.map(value => [value.segment, value.index, value.input === text, value.isWordLike]),
      containing: [0, 3, 4, 5, 21, 22, 100, -1, NaN, 4.5, undefined].map(index => segments.containing(index)?.index),
    };
  });
  const localeTags = ["en", "en-GB", "en-AU", "en-IN", "es-MX", "fr-CA", "de", "ja", "zh-TW", "ko", "ar-EG", "he", "fa", "hi", "pt-BR", "ms", "pa-Arab", "ja-u-hc-h12", "en-u-hc-h23"];
  result.localeData = Object.fromEntries(localeTags.map(tag => {
    const locale = new Intl.Locale(tag);
    const { hourCycle, hour12 } = new Intl.DateTimeFormat(tag, { hour: "numeric", timeZone: "UTC" }).resolvedOptions();
    const dayPeriod = new Intl.DateTimeFormat(tag, { hour: "numeric", timeZone: "UTC" }).formatToParts(date).some(part => part.type === "dayPeriod");
    return [tag, { hourCycle: locale.hourCycle ?? null, hourCycles: locale.getHourCycles(), textInfo: locale.getTextInfo(), weekInfo: locale.getWeekInfo(), formatted: { hourCycle, hour12, dayPeriod } }];
  }));
  result.firstDayKeyword = new Intl.Locale("ar-u-fw-mon").getWeekInfo();
  result.localeAccessors = ["languageOf", "regionOf", "scriptOf", "calendarOf", "hourCycleOf", "numberingSystemOf"].filter(name => name in Intl.Locale.prototype);
  result.currencyDigits = Object.fromEntries(["USD", "JPY", "KRW", "BHD", "KWD", "CLF", "HUF", "IDR", "TWD", "CHF", "IQD", "XOF"].map(currency => {
    const formatter = new Intl.NumberFormat("en-US", { style: "currency", currency, currencyDisplay: "code" });
    const { minimumFractionDigits, maximumFractionDigits } = formatter.resolvedOptions();
    return [currency, [minimumFractionDigits, maximumFractionDigits, formatter.format(1234.5678).replace(/\s/g, " ")]];
  }));
  result.fractionDigits = ([
    { style: "currency", currency: "KWD", minimumFractionDigits: 1 }, { style: "currency", currency: "EUR", maximumFractionDigits: 1 },
    { style: "currency", currency: "EUR", maximumFractionDigits: 0 }, { style: "currency", currency: "JPY", minimumFractionDigits: 3 },
    { maximumFractionDigits: 1 }, { style: "percent", maximumFractionDigits: 2 }, { minimumFractionDigits: 3, maximumFractionDigits: 1 }, { minimumFractionDigits: 101 },
  ] satisfies Intl.NumberFormatOptions[]).map(options => outcome(() => {
    const formatter = new Intl.NumberFormat("en-US", options);
    const { minimumFractionDigits, maximumFractionDigits } = formatter.resolvedOptions();
    return [minimumFractionDigits, maximumFractionDigits, formatter.format(1.23456)];
  }));
  result.boundFormat = outcome(() => {
    const dateFormatter = new Intl.DateTimeFormat("en-GB", { dateStyle: "short", timeZone: "UTC" });
    const numberFormatter = new Intl.NumberFormat("de");
    const collator = new Intl.Collator("de", { numeric: true });
    const dateFormat = dateFormatter.format, numberFormat = numberFormatter.format, compare = collator.compare;
    return { date: dateFormat(date), number: numberFormat(1234.5), compare: Math.sign(compare("2", "10")),
      stable: dateFormat === dateFormatter.format && numberFormat === numberFormatter.format && compare === collator.compare };
  });
  return result;
}
