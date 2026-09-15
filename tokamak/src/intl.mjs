import {
  intlCanonicalLocales,
  intlCollatorCompare,
  intlDateTime,
  intlDateTimeParts,
  intlDisplayName,
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
  if (typeof locales === "string") return [locales];
  if (locales === null || typeof locales[Symbol.iterator] !== "function") throw new TypeError("Invalid language tag");
  return [...locales].map(value => String(value));
}

function canonicalLocales(locales) {
  return JSON.parse(intlCanonicalLocales(JSON.stringify(localeList(locales))));
}

function firstLocale(locales) {
  return canonicalLocales(locales)[0] ?? "en-US";
}

function optionsObject(options) {
  if (options === undefined || options === null) return {};
  if (typeof options !== "object" && typeof options !== "function") throw new TypeError("Options must be an object");
  return options;
}

function optionsJson(options) {
  return JSON.stringify(optionsObject(options), (_, value) => typeof value === "bigint" ? String(value) : value);
}

function localeInfo(locale) {
  return JSON.parse(intlLocaleInfo(locale, "{}"));
}

function localeDefaultHourCycle(locale) {
  const info = localeInfo(locale);
  if (info.hourCycle) return info.hourCycle;
  return /^en-US|en-CA|en-PH|es-US|fr-CA/.test(locale) ? "h12" : "h23";
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

function hasTimeFields(options) {
  return options.timeStyle !== undefined || ["dayPeriod", "hour", "minute", "second", "fractionalSecondDigits", "timeZoneName"].some(field => options[field] !== undefined);
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

function epochMilliseconds(value) {
  const date = value === undefined ? new Date() : value instanceof Date ? value : new Date(value);
  const milliseconds = date.getTime();
  if (Number.isNaN(milliseconds)) throw new RangeError("Invalid time value");
  return milliseconds;
}

function dateResolvedOptions(locale, options) {
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
    if (options.timeStyle !== undefined) {
      const hourCycle = options.hourCycle ?? (options.hour12 === undefined ? localeDefaultHourCycle(locale) : options.hour12 ? "h12" : "h23");
      result.hourCycle = hourCycle;
      result.hour12 = hourCycle === "h11" || hourCycle === "h12";
    }
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
  if (hasTime) {
    const hourCycle = options.hourCycle ?? (options.hour12 === undefined ? localeDefaultHourCycle(locale) : options.hour12 ? "h12" : "h23");
    result.hourCycle = hourCycle;
    result.hour12 = hourCycle === "h11" || hourCycle === "h12";
  }
  return result;
}

class DateTimeFormat {
  constructor(locales, options) {
    this.__locales = canonicalLocales(locales);
    this.__locale = this.__locales[0] ?? "en-US";
    this.__options = dateOptions(options);
    if (hasTimeFields(this.__options) && this.__options.hourCycle === undefined && this.__options.hour12 === undefined) {
      this.__options.hourCycle = localeDefaultHourCycle(this.__locale);
    }
  }
  format(value = new Date()) {
    return intlDateTime(epochMilliseconds(value), JSON.stringify(this.__locales), optionsJson(this.__options));
  }
  formatToParts(value = new Date()) {
    return formatParts(intlDateTimeParts, epochMilliseconds(value), JSON.stringify(this.__locales), optionsJson(this.__options));
  }
  formatRange(start, end) {
    const first = this.format(start);
    const second = this.format(end);
    return first === second ? first : first + " – " + second;
  }
  formatRangeToParts(start, end) {
    const first = this.formatToParts(start);
    const second = this.formatToParts(end);
    if (this.format(start) === this.format(end)) return first.map(part => ({ ...part, source: "shared" }));
    return [...first.map(part => ({ ...part, source: "startRange" })), { type: "literal", value: " – ", source: "shared" }, ...second.map(part => ({ ...part, source: "endRange" }))];
  }
  resolvedOptions() { return dateResolvedOptions(this.__locale, this.__options); }
  static supportedLocalesOf(locales) { return canonicalLocales(locales); }
}

function numberResolvedOptions(locale, options) {
  const style = options.style;
  const currencyDigits = style === "currency" && options.currency === "JPY" ? 0 : style === "currency" ? 2 : 0;
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
  result.minimumFractionDigits = options.minimumFractionDigits ?? (style === "currency" ? currencyDigits : 0);
  result.maximumFractionDigits = options.maximumFractionDigits ?? Math.max(style === "currency" ? currencyDigits : style === "percent" ? 0 : 3, result.minimumFractionDigits);
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
  constructor(locales, options) {
    this.__locales = canonicalLocales(locales);
    this.__locale = this.__locales[0] ?? "en-US";
    this.__options = numberOptions(options);
  }
  format(value) { return intlNumber(Number(value), JSON.stringify(this.__locales), optionsJson(this.__options)); }
  formatToParts(value) { return formatParts(intlNumberParts, Number(value), JSON.stringify(this.__locales), optionsJson(this.__options)); }
  formatRange(start, end) {
    const first = this.format(start);
    const second = this.format(end);
    return first === second ? first : first + " – " + second;
  }
  formatRangeToParts(start, end) {
    const first = this.format(start);
    const second = this.format(end);
    return first === second ? this.formatToParts(start).map(part => ({ ...part, source: "shared" })) : [{ type: "literal", value: first, source: "startRange" }, { type: "literal", value: " – ", source: "shared" }, { type: "literal", value: second, source: "endRange" }];
  }
  resolvedOptions() { return numberResolvedOptions(this.__locale, this.__options); }
  static supportedLocalesOf(locales) { return canonicalLocales(locales); }
}

class PluralRules {
  constructor(locales, options) {
    this.__locales = canonicalLocales(locales);
    this.__locale = this.__locales[0] ?? "en-US";
    this.__options = pluralOptions(options);
  }
  select(value) { return intlPlural(Number(value), JSON.stringify(this.__locales), this.__options.type); }
  selectRange(start, end) { return intlPluralRange(Number(start), Number(end), JSON.stringify(this.__locales), this.__options.type); }
  resolvedOptions() {
    return {
      locale: this.__locale,
      type: this.__options.type,
      notation: "standard",
      minimumIntegerDigits: 1,
      minimumFractionDigits: 0,
      maximumFractionDigits: 3,
      pluralCategories: JSON.parse(intlPluralCategories(JSON.stringify(this.__locales), this.__options.type)),
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
  constructor(tag, options) {
    this.__tag = localeTagWithOptions(tag, options);
    this.__info = localeInfo(this.__tag);
  }
  get baseName() { return this.__info.baseName; }
  get language() { return this.__info.language; }
  get region() { return this.__info.region ?? undefined; }
  get script() { return this.__info.script ?? undefined; }
  get calendar() { return this.__info.calendar; }
  get caseFirst() { return this.__tag.match(/-kf-([a-z]+)/)?.[1] ?? undefined; }
  get collation() { return this.__tag.match(/-co-([a-z-]+)/)?.[1] ?? undefined; }
  get hourCycle() { return this.__info.hourCycle ?? localeDefaultHourCycle(this.__tag); }
  get numeric() { return /-kn(?:-|$)/.test(this.__tag); }
  toString() { return this.__info.string; }
  maximize() { return new Locale(this.__info.maximize); }
  minimize() { return new Locale(this.__info.minimize); }
  getTextInfo() { return { direction: ["ar", "fa", "he", "ur", "ps", "sd", "ug", "yi"].includes(this.language) ? "rtl" : "ltr" }; }
  getWeekInfo() { return { firstDay: ["US", "CA", "JP", "PH"].includes(this.region) ? 7 : 1, weekend: [6, 7] }; }
  languageOf() { return this.language; }
  regionOf() { return this.region; }
  scriptOf() { return this.script; }
  calendarOf() { return this.calendar; }
  caseFirstOf() { return this.caseFirst; }
  collationOf() { return this.collation; }
  hourCycleOf() { return this.hourCycle; }
  numberingSystemOf() { return this.numberingSystem; }
}

class ListFormat {
  constructor(locales, options) {
    this.__locales = canonicalLocales(locales);
    this.__locale = this.__locales[0] ?? "en-US";
    this.__options = listOptions(options);
  }
  format(list) { return intlList(JSON.stringify([...list].map(value => String(value))), JSON.stringify(this.__locales), optionsJson(this.__options)); }
  formatToParts(list) { return formatParts(intlListParts, JSON.stringify([...list].map(value => String(value))), JSON.stringify(this.__locales), optionsJson(this.__options)); }
  resolvedOptions() { return { locale: this.__locale, ...this.__options }; }
  static supportedLocalesOf(locales) { return canonicalLocales(locales); }
}

class RelativeTimeFormat {
  constructor(locales, options) {
    this.__locales = canonicalLocales(locales);
    this.__locale = this.__locales[0] ?? "en-US";
    this.__options = relativeOptions(options);
  }
  format(value, unit) { return intlRelative(Number(value), String(unit), JSON.stringify(this.__locales), optionsJson(this.__options)); }
  formatToParts(value, unit) { return JSON.parse(intlRelativeParts(Number(value), String(unit), JSON.stringify(this.__locales), optionsJson(this.__options))); }
  resolvedOptions() { return { locale: this.__locale, ...this.__options, numberingSystem: localeInfo(this.__locale).numberingSystem }; }
  static supportedLocalesOf(locales) { return canonicalLocales(locales); }
}

class Collator {
  constructor(locales, options) {
    this.__locales = canonicalLocales(locales);
    this.__locale = this.__locales[0] ?? "en-US";
    const source = optionsObject(options);
    this.__options = {
      usage: source.usage ?? "sort",
      sensitivity: source.sensitivity ?? "variant",
      ignorePunctuation: source.ignorePunctuation ?? false,
      collation: source.collation ?? "default",
      numeric: source.numeric ?? false,
      caseFirst: source.caseFirst ?? "false",
    };
    this.compare = (left, right) => intlCollatorCompare(String(left), String(right), JSON.stringify(this.__locales), optionsJson(this.__options));
  }
  resolvedOptions() { return { locale: this.__locale, ...this.__options }; }
  static supportedLocalesOf(locales) { return canonicalLocales(locales); }
}

class Segmenter {
  constructor(locales, options) {
    this.__locales = canonicalLocales(locales);
    this.__locale = this.__locales[0] ?? "en-US";
    this.__granularity = optionsObject(options).granularity ?? "grapheme";
    if (!["grapheme", "word", "sentence"].includes(this.__granularity)) throw new RangeError("Invalid granularity");
  }
  segment(input) {
    const source = String(input);
    const values = JSON.parse(intlSegment(source, JSON.stringify(this.__locales), this.__granularity));
    return {
      containing(index) {
        const position = Number(index);
        return values.find((value, valueIndex) => position >= value.index && (values[valueIndex + 1]?.index ?? Infinity) > position);
      },
      [Symbol.iterator]: function* () { yield* values; },
    };
  }
  resolvedOptions() { return { locale: this.__locale, granularity: this.__granularity }; }
  static supportedLocalesOf(locales) { return canonicalLocales(locales); }
}

class DisplayNames {
  constructor(locales, options) {
    this.__locales = canonicalLocales(locales);
    this.__locale = this.__locales[0] ?? "en-US";
    const source = optionsObject(options);
    this.__options = {
      style: source.style ?? "long",
      type: source.type,
      fallback: source.fallback ?? "code",
      languageDisplay: source.languageDisplay ?? "dialect",
    };
    if (!this.__options.type) throw new TypeError("DisplayNames type is required");
  }
  of(value) { return intlDisplayName(String(value), JSON.stringify(this.__locales), optionsJson(this.__options)); }
  resolvedOptions() { return { locale: this.__locale, ...this.__options }; }
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

export { intl };
