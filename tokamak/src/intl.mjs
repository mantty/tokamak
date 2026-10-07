import { hostObjectKinds, markHostObject } from "./globals/objects.mjs";
import {
  intlCanonicalLocales,
  intlCollatorCompare,
  intlDateTime,
  intlDateTimeParts,
  intlDisplayName,
  intlFractionDigits,
  intlHourCycle,
  intlList,
  intlListParts,
  intlLocaleInfo,
  intlNumber,
  intlNumberParts,
  intlPlural,
  intlPluralCategories,
  intlPluralRange,
  intlRelative,
  intlRelativeParts,
  intlSegment,
} from "tokamak:host";

const dateFields = ["weekday", "era", "year", "month", "day", "dayPeriod", "hour", "minute", "second", "fractionalSecondDigits", "timeZoneName"];
const numberFields = ["minimumIntegerDigits", "minimumFractionDigits", "maximumFractionDigits", "minimumSignificantDigits", "maximumSignificantDigits", "useGrouping", "notation", "compactDisplay", "signDisplay", "roundingIncrement", "roundingMode", "roundingPriority", "trailingZeroDisplay", "currencySign", "currencyDisplay", "unitDisplay"];

function localeList(locales) {
  if (locales === undefined) return [];
  if (typeof locales === "string" || hostObjectKinds.get(locales) === "Intl.Locale") return [String(locales)];
  if (locales === null) throw new TypeError("Invalid language tag");
  const source = Object(locales), values = [];
  const length = Math.min(Math.max(Math.trunc(+source.length) || 0, 0), Number.MAX_SAFE_INTEGER);
  for (let i = 0; i < length; i++) {
    if (!(i in source)) continue;
    const value = source[i];
    if (typeof value !== "string" && (value === null || (typeof value !== "object" && typeof value !== "function"))) throw new TypeError("Invalid language tag");
    values.push(String(value));
  }
  return values;
}

function canonicalLocales(locales) {
  return JSON.parse(intlCanonicalLocales(JSON.stringify(localeList(locales))));
}

function firstLocale(locales) {
  return canonicalLocales(locales)[0] ?? "en-US";
}

function optionsObject(options) {
  if (options === undefined) return {};
  if (options === null) throw new TypeError("Options must not be null");
  return Object(options);
}

function optionsJson(options) {
  return JSON.stringify(optionsObject(options), (_, value) => typeof value === "bigint" ? String(value) : value);
}

// Marks `object` as a host object and returns its canonical locales, as the JSON
// the host takes, and the locale formatting uses.
function initIntlObject(object, locales) {
  markHostObject(object);
  const list = canonicalLocales(locales);
  return { json: JSON.stringify(list), locale: list[0] ?? "en-US" };
}

function localeInfo(locale) {
  return JSON.parse(intlLocaleInfo(locale));
}

function dateOptions(options) {
  const source = optionsObject(options);
  const result = {};
  for (const field of dateFields) if (source[field] !== undefined) result[field] = source[field];
  if (source.dateStyle !== undefined) result.dateStyle = source.dateStyle;
  if (source.timeStyle !== undefined) result.timeStyle = source.timeStyle;
  result.timeZone = source.timeZone ?? "UTC";
  if (source.calendar !== undefined) result.calendar = source.calendar;
  if (source.numberingSystem !== undefined) result.numberingSystem = source.numberingSystem;
  if (source.hourCycle !== undefined) result.hourCycle = source.hourCycle;
  if (source.hour12 !== undefined) result.hour12 = source.hour12;
  return result;
}

function numberOptions(options) {
  const source = optionsObject(options);
  const result = {};
  for (const field of numberFields) if (source[field] !== undefined) result[field] = source[field];
  result.style = source.style ?? "decimal";
  if (result.style === "currency") {
    if (source.currency === undefined) throw new TypeError("Currency is required for currency style");
    result.currency = String(source.currency).toUpperCase();
  }
  if (result.style === "unit") {
    if (source.unit === undefined) throw new TypeError("Unit is required for unit style");
    result.unit = String(source.unit);
  }
  return result;
}

function pluralOptions(options) {
  const source = optionsObject(options);
  return { type: source.type ?? "cardinal" };
}

function listOptions(options) {
  const source = optionsObject(options);
  return { type: source.type ?? "conjunction", style: source.style ?? "long" };
}

function relativeOptions(options) {
  const source = optionsObject(options);
  return { numeric: source.numeric ?? "always", style: source.style ?? "long" };
}

function formatParts(host, ...args) {
  return JSON.parse(host(...args)).map(([type, value]) => ({ type, value }));
}

function joinRange(first, second) {
  return first === second ? first : first + " – " + second;
}

function epochMilliseconds(value) {
  const milliseconds = value === undefined ? Date.now() : +value;
  if (!Number.isFinite(milliseconds) || Math.abs(milliseconds) > 8_640_000_000_000_000) throw new RangeError("Invalid time value");
  return Math.trunc(milliseconds);
}

function dateResolvedOptions({ locale, json }, options) {
  const info = localeInfo(locale);
  const result = {
    locale,
    calendar: options.calendar ?? info.calendar,
    numberingSystem: options.numberingSystem ?? info.numberingSystem,
    timeZone: options.timeZone,
  };
  if (options.dateStyle !== undefined || options.timeStyle !== undefined) {
    if (options.dateStyle !== undefined) result.dateStyle = options.dateStyle;
    if (options.timeStyle !== undefined) result.timeStyle = options.timeStyle;
    if (options.timeStyle !== undefined) resolveHourCycle(result, json, options);
    return result;
  }
  const hasDate = dateFields.some(field => ["weekday", "era", "year", "month", "day"].includes(field) && options[field] !== undefined);
  const hasTime = dateFields.some(field => ["dayPeriod", "hour", "minute", "second", "fractionalSecondDigits", "timeZoneName"].includes(field) && options[field] !== undefined);
  if (!hasDate && !hasTime) {
    result.year = "numeric";
    result.month = "numeric";
    result.day = "numeric";
  } else {
    for (const field of dateFields) if (options[field] !== undefined) result[field] = options[field];
  }
  if (hasTime) resolveHourCycle(result, json, options);
  return result;
}

function resolveHourCycle(result, locales, options) {
  const hourCycle = options.hourCycle ?? (options.hour12 === undefined ? intlHourCycle(locales) : options.hour12 ? "h12" : "h23");
  result.hourCycle = hourCycle;
  result.hour12 = hourCycle === "h11" || hourCycle === "h12";
}

class DateTimeFormat {
  #locales;
  #options;
  #format;
  constructor(locales, options) {
    this.#locales = initIntlObject(this, locales);
    this.#options = dateOptions(options);
    this.#format = value => intlDateTime(epochMilliseconds(value), this.#locales.json, optionsJson(this.#options));
  }
  get format() { return this.#format; }
  formatToParts(value = new Date()) {
    return formatParts(intlDateTimeParts, epochMilliseconds(value), this.#locales.json, optionsJson(this.#options));
  }
  formatRange(start, end) { return joinRange(this.format(start), this.format(end)); }
  formatRangeToParts(start, end) {
    const first = this.formatToParts(start);
    const second = this.formatToParts(end);
    if (this.format(start) === this.format(end)) return first.map(part => ({ ...part, source: "shared" }));
    return [...first.map(part => ({ ...part, source: "startRange" })), { type: "literal", value: " – ", source: "shared" }, ...second.map(part => ({ ...part, source: "endRange" }))];
  }
  resolvedOptions() { return dateResolvedOptions(this.#locales, this.#options); }
  static supportedLocalesOf(locales) { return canonicalLocales(locales); }
}

function numberResolvedOptions(locale, options, [minimumFractionDigits, maximumFractionDigits]) {
  const style = options.style;
  const result = {
    locale,
    numberingSystem: localeInfo(locale).numberingSystem,
    style,
  };
  if (style === "currency") {
    result.currency = options.currency;
    result.currencyDisplay = options.currencyDisplay ?? "symbol";
    result.currencySign = options.currencySign ?? "standard";
  }
  if (style === "unit") result.unit = options.unit;
  result.minimumIntegerDigits = options.minimumIntegerDigits ?? 1;
  result.minimumFractionDigits = minimumFractionDigits;
  result.maximumFractionDigits = maximumFractionDigits;
  result.useGrouping = options.useGrouping ?? "auto";
  result.notation = options.notation ?? "standard";
  result.signDisplay = options.signDisplay ?? "auto";
  result.roundingIncrement = options.roundingIncrement ?? 1;
  result.roundingMode = options.roundingMode ?? "halfExpand";
  result.roundingPriority = options.roundingPriority ?? "auto";
  result.trailingZeroDisplay = options.trailingZeroDisplay ?? "auto";
  if (options.minimumSignificantDigits !== undefined) result.minimumSignificantDigits = options.minimumSignificantDigits;
  if (options.maximumSignificantDigits !== undefined) result.maximumSignificantDigits = options.maximumSignificantDigits;
  return result;
}

class NumberFormat {
  #locales;
  #options;
  #fractionDigits;
  #format;
  constructor(locales, options) {
    this.#locales = initIntlObject(this, locales);
    this.#options = numberOptions(options);
    this.#fractionDigits = intlFractionDigits(optionsJson(this.#options));
    this.#format = value => intlNumber(Number(value), this.#locales.json, optionsJson(this.#options));
  }
  get format() { return this.#format; }
  formatToParts(value) { return formatParts(intlNumberParts, Number(value), this.#locales.json, optionsJson(this.#options)); }
  formatRange(start, end) { return joinRange(this.format(start), this.format(end)); }
  formatRangeToParts(start, end) {
    const first = this.format(start);
    const second = this.format(end);
    return first === second ? this.formatToParts(start).map(part => ({ ...part, source: "shared" })) : [{ type: "literal", value: first, source: "startRange" }, { type: "literal", value: " – ", source: "shared" }, { type: "literal", value: second, source: "endRange" }];
  }
  resolvedOptions() { return numberResolvedOptions(this.#locales.locale, this.#options, this.#fractionDigits); }
  static supportedLocalesOf(locales) { return canonicalLocales(locales); }
}

class PluralRules {
  #locales;
  #options;
  constructor(locales, options) {
    this.#locales = initIntlObject(this, locales);
    this.#options = pluralOptions(options);
  }
  select(value) { return intlPlural(Number(value), this.#locales.json, this.#options.type); }
  selectRange(start, end) { return intlPluralRange(Number(start), Number(end), this.#locales.json, this.#options.type); }
  resolvedOptions() {
    return {
      locale: this.#locales.locale,
      type: this.#options.type,
      notation: "standard",
      minimumIntegerDigits: 1,
      minimumFractionDigits: 0,
      maximumFractionDigits: 3,
      pluralCategories: JSON.parse(intlPluralCategories(this.#locales.json, this.#options.type)),
      roundingIncrement: 1,
      roundingMode: "halfExpand",
      roundingPriority: "auto",
      trailingZeroDisplay: "auto",
    };
  }
  static supportedLocalesOf(locales) { return canonicalLocales(locales); }
}

function localeTagWithOptions(tag, options) {
  const base = String(tag);
  const source = optionsObject(options);
  const values = [];
  if (source.calendar !== undefined) values.push(["ca", String(source.calendar)]);
  if (source.collation !== undefined) values.push(["co", String(source.collation)]);
  if (source.hourCycle !== undefined) values.push(["hc", String(source.hourCycle)]);
  if (source.caseFirst !== undefined) values.push(["kf", String(source.caseFirst)]);
  if (source.numeric !== undefined) values.push(["kn", source.numeric ? "" : "false"]);
  if (source.numberingSystem !== undefined) values.push(["nu", String(source.numberingSystem)]);
  if (values.length === 0) return firstLocale(base);
  const extension = values.flatMap(([key, value]) => value ? [key, value] : [key]).join("-");
  return firstLocale(base + (base.toLowerCase().includes("-u-") ? "-" : "-u-") + extension);
}

class Locale {
  #tag;
  #info;
  constructor(tag, options) {
    markHostObject(this, "Intl.Locale");
    this.#tag = localeTagWithOptions(tag, options);
    this.#info = localeInfo(this.#tag);
  }
  get baseName() { return this.#info.baseName; }
  get language() { return this.#info.language; }
  get region() { return this.#info.region ?? undefined; }
  get script() { return this.#info.script ?? undefined; }
  get calendar() { return this.#info.calendar; }
  get caseFirst() { return this.#tag.match(/-kf-([a-z]+)/)?.[1] ?? undefined; }
  get collation() { return this.#tag.match(/-co-([a-z-]+)/)?.[1] ?? undefined; }
  get hourCycle() { return this.#info.hourCycle ?? undefined; }
  get numeric() { return /-kn(?:-|$)/.test(this.#tag); }
  toString() { return this.#info.string; }
  maximize() { return new Locale(this.#info.maximize); }
  minimize() { return new Locale(this.#info.minimize); }
  getHourCycles() { return [intlHourCycle(JSON.stringify([this.#tag]))]; }
  getTextInfo() { return { direction: this.#info.direction }; }
  getWeekInfo() { return { firstDay: this.#info.firstDay, weekend: this.#info.weekend }; }
  languageOf() { return this.language; }
  regionOf() { return this.region; }
  scriptOf() { return this.script; }
  calendarOf() { return this.calendar; }
  caseFirstOf() { return this.caseFirst; }
  collationOf() { return this.collation; }
  hourCycleOf() { return this.hourCycle; }
  numberingSystemOf() { return this.numberingSystem; }
}

function listJson(list) {
  return JSON.stringify([...list].map(value => String(value)));
}

class ListFormat {
  #locales;
  #options;
  constructor(locales, options) {
    this.#locales = initIntlObject(this, locales);
    this.#options = listOptions(options);
  }
  format(list) { return intlList(listJson(list), this.#locales.json, optionsJson(this.#options)); }
  formatToParts(list) { return formatParts(intlListParts, listJson(list), this.#locales.json, optionsJson(this.#options)); }
  resolvedOptions() { return { locale: this.#locales.locale, ...this.#options }; }
  static supportedLocalesOf(locales) { return canonicalLocales(locales); }
}

class RelativeTimeFormat {
  #locales;
  #options;
  constructor(locales, options) {
    this.#locales = initIntlObject(this, locales);
    this.#options = relativeOptions(options);
  }
  format(value, unit) { return intlRelative(Number(value), String(unit), this.#locales.json, optionsJson(this.#options)); }
  formatToParts(value, unit) { return JSON.parse(intlRelativeParts(Number(value), String(unit), this.#locales.json, optionsJson(this.#options))); }
  resolvedOptions() { return { locale: this.#locales.locale, ...this.#options, numberingSystem: localeInfo(this.#locales.locale).numberingSystem }; }
  static supportedLocalesOf(locales) { return canonicalLocales(locales); }
}

class Collator {
  #locales;
  #options;
  #compare;
  constructor(locales, options) {
    this.#locales = initIntlObject(this, locales);
    const source = optionsObject(options);
    this.#options = {
      usage: source.usage ?? "sort",
      sensitivity: source.sensitivity ?? "variant",
      ignorePunctuation: source.ignorePunctuation ?? false,
      collation: source.collation ?? "default",
      numeric: source.numeric ?? false,
      caseFirst: source.caseFirst ?? "false",
    };
    this.#compare = (left, right) => intlCollatorCompare(String(left), String(right), this.#locales.json, optionsJson(this.#options));
  }
  get compare() { return this.#compare; }
  resolvedOptions() { return { locale: this.#locales.locale, ...this.#options }; }
  static supportedLocalesOf(locales) { return canonicalLocales(locales); }
}

class Segmenter {
  #locales;
  #granularity;
  constructor(locales, options) {
    this.#locales = initIntlObject(this, locales);
    this.#granularity = optionsObject(options).granularity ?? "grapheme";
    if (!["grapheme", "word", "sentence"].includes(this.#granularity)) throw new RangeError("Invalid granularity");
  }
  segment(input) {
    const source = String(input);
    const values = JSON.parse(intlSegment(source, this.#locales.json, this.#granularity));
    return {
      containing(index) {
        const position = Number(index);
        return values.find((value, valueIndex) => position >= value.index && (values[valueIndex + 1]?.index ?? Infinity) > position);
      },
      [Symbol.iterator]: function* () { yield* values; },
    };
  }
  resolvedOptions() { return { locale: this.#locales.locale, granularity: this.#granularity }; }
  static supportedLocalesOf(locales) { return canonicalLocales(locales); }
}

class DisplayNames {
  #locales;
  #options;
  constructor(locales, options) {
    this.#locales = initIntlObject(this, locales);
    const source = optionsObject(options);
    this.#options = {
      style: source.style ?? "long",
      type: source.type,
      fallback: source.fallback ?? "code",
      languageDisplay: source.languageDisplay ?? "dialect",
    };
    if (!this.#options.type) throw new TypeError("DisplayNames type is required");
  }
  of(value) { return intlDisplayName(String(value), this.#locales.json, optionsJson(this.#options)); }
  resolvedOptions() { return { locale: this.#locales.locale, ...this.#options }; }
}

const supportedValues = {
  calendar: ["buddhist", "chinese", "coptic", "dangi", "ethioaa", "ethiopic", "gregory", "hebrew", "indian", "islamic", "islamic-civil", "islamic-rgsa", "islamic-tbla", "islamic-umalqura", "iso8601", "japanese", "persian", "roc"],
  numberingSystem: ["adlm", "ahom", "arab", "arabext", "bali", "beng", "bhks", "brah", "cakm", "cham", "deva", "diak", "fullwide", "gong", "gonm", "gujr", "guru", "hanidec", "hmng", "hmnp", "java", "kali", "khmr", "knda", "lana", "lanatham", "laoo", "latn", "lepc", "limb", "mathbold", "mathdbl", "mathmono", "mathsanb", "mathsans", "mlym", "modi", "mong", "mroo", "mtei", "mymr", "mymrepka", "mymrpao", "nagm", "newa", "nkoo", "olck", "orya", "osma", "rohg", "saur", "segment", "shrd", "sind", "sinh", "sora", "sund", "takr", "talu", "taml", "tamldec", "telu", "thai", "tibt", "tirh", "tnsa", "vaii", "wara", "wcho"],
  timeZone: ["Africa/Abidjan", "Africa/Accra", "Africa/Addis_Ababa", "America/Adak", "America/Anchorage", "America/Argentina/Buenos_Aires", "America/Chicago", "America/Denver", "America/Los_Angeles", "America/New_York", "America/Phoenix", "America/Sao_Paulo", "America/Toronto", "Asia/Kolkata", "Asia/Tokyo", "Australia/Sydney", "Europe/Berlin", "Europe/London", "Europe/Paris", "Pacific/Auckland"],
};

const intl = {
  Collator,
  DateTimeFormat,
  DisplayNames,
  ListFormat,
  Locale,
  NumberFormat,
  PluralRules,
  RelativeTimeFormat,
  Segmenter,
  getCanonicalLocales(locales) { return canonicalLocales(locales); },
  supportedValuesOf(key) {
    if (!(key in supportedValues)) throw new RangeError("Invalid key");
    return [...supportedValues[key]];
  },
};

function dateLocaleString(value, locales, options, required, defaults) {
  const milliseconds = Date.prototype.getTime.call(value);
  if (Number.isNaN(milliseconds)) return "Invalid Date";
  const formatOptions = Object.create(optionsObject(options));
  if (required === "date" && formatOptions.timeStyle !== undefined) throw new TypeError("timeStyle is not valid for toLocaleDateString");
  if (required === "time" && formatOptions.dateStyle !== undefined) throw new TypeError("dateStyle is not valid for toLocaleTimeString");
  const dateFields = ["weekday", "year", "month", "day"], timeFields = ["hour", "minute", "second", "fractionalSecondDigits"];
  const fields = required === "date" ? dateFields : required === "time" ? timeFields : [...dateFields, ...timeFields];
  if (formatOptions.dateStyle === undefined && formatOptions.timeStyle === undefined && fields.every(field => formatOptions[field] === undefined)) {
    if (defaults !== "time") for (const field of ["year", "month", "day"]) formatOptions[field] = "numeric";
    if (defaults !== "date") for (const field of ["hour", "minute", "second"]) formatOptions[field] = "numeric";
  }
  return new DateTimeFormat(locales, formatOptions).format(milliseconds);
}

export function installIntlGlobals() {
  globalThis.Intl = intl;
  Object.defineProperties(Date.prototype, {
    toLocaleString: { configurable: true, writable: true, value: function toLocaleString(locales, options) { return dateLocaleString(this, locales, options, "any", "all"); } },
    toLocaleDateString: { configurable: true, writable: true, value: function toLocaleDateString(locales, options) { return dateLocaleString(this, locales, options, "date", "date"); } },
    toLocaleTimeString: { configurable: true, writable: true, value: function toLocaleTimeString(locales, options) { return dateLocaleString(this, locales, options, "time", "time"); } },
  });
  Object.defineProperty(Number.prototype, "toLocaleString", { configurable: true, writable: true,
    value: function toLocaleString(locales, options) { return new NumberFormat(locales, options).format(Number.prototype.valueOf.call(this)); } });
  Object.defineProperty(String.prototype, "localeCompare", { configurable: true, writable: true,
    value: function localeCompare(other, locales, options) {
      if (this == null || typeof this === "symbol" || typeof other === "symbol") throw new TypeError("Cannot convert value to string");
      return new Collator(locales, options).compare(String(this), String(other));
    } });
}

export { intl };
