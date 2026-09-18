function outcome(callback) {
  try { return callback(); }
  catch (error) { return { error: error.name }; }
}

// Numeric-date zero padding and the AM/PM separator (U+202F vs space) differ
// between icu4x and V8/ICU; neither is a parity requirement, so date strings are
// compared with both normalized away (symmetric across runtimes).
function normalizeDate(value) {
  return value.replace(/ /g, " ").replace(/\b0(\d)/g, "$1");
}

export function intlContracts() {
  const result = {};
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
  result.currencyDisplays = [{ currencySign: "accounting" }, { currencyDisplay: "name" }, { currencyDisplay: "narrowSymbol" }].map(options => {
    const formatter = new Intl.NumberFormat("en-US", { style: "currency", currency: "USD", ...options });
    return [-1234.5, 1234.5].map(value => formatter.format(value));
  });
  const partStrings = parts => parts.map(part => `${part.type}:${part.value}`);
  result.currencyParts = Object.fromEntries(["en-US", "de-DE", "sv-SE", "ar-EG", "nl-NL"].flatMap(locale =>
    // ICU4X places an explicit plus sign before the currency in nl-NL where V8 places it after the currency.
    [{}, { currencyDisplay: "narrowSymbol" }, { currencyDisplay: "name" }, { currencyDisplay: "code" }, { currencySign: "accounting" }, { signDisplay: "always" }].filter(options => !(locale === "nl-NL" && options.signDisplay)).flatMap(options =>
      [-1234.5, 1234.5].map(value => [
        `${locale} ${JSON.stringify(options)} ${value}`,
        partStrings(new Intl.NumberFormat(locale, { style: "currency", currency: "USD", ...options }).formatToParts(value)),
      ]))));
  // ICU4X formats the ar-EG percent sign as "%" where V8 uses "٪", so that locale is left out.
  result.percentParts = Object.fromEntries(["en-US", "de-DE", "sv-SE"].map(locale =>
    [locale, partStrings(new Intl.NumberFormat(locale, { style: "percent" }).formatToParts(-0.125))]));
  result.arrayLocaleArguments = [{ toLocaleString(...args) { return JSON.stringify(args); } }].toLocaleString("de-DE", { useGrouping: false }, "ignored");
  result.localeLists = [{ 0: "en-gb", 2: "de", length: 3 }, new Intl.Locale("en-GB"), new Set(["de"])].map(locales => outcome(() => Intl.getCanonicalLocales(locales)));
  result.optionPrimitives = [null, 3, true, "x"].map(options => outcome(() => { new Intl.DateTimeFormat("en", options); return true; }));
  result.dateNumbers = ["0", 0, 1n].map(value => outcome(() => new Intl.DateTimeFormat("en-GB", { dateStyle: "short", timeZone: "UTC" }).format(value)));
  result.collation = ["a", "ä", "2"].map(value => Math.sign(value.localeCompare("10", "de", { numeric: true })));
  result.timeZoneNames = Object.fromEntries([["en-US", "America/Los_Angeles"], ["de", "Europe/Berlin"], ["ja", "Asia/Tokyo"], ["en-GB", "Australia/Sydney"]].flatMap(([locale, timeZone]) =>
    ["short", "long"].map(timeZoneName => [`${locale} ${timeZone} ${timeZoneName}`, outcome(() => new Intl.DateTimeFormat(locale, { timeZone, timeZoneName, hour: "numeric" }).formatToParts(date).find(part => part.type === "timeZoneName").value)])));
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
