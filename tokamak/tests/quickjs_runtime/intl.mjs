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
