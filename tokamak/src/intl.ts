import { hostObjectKinds, isIterable, isObject, markHostObject, objectOf } from "./globals/objects.ts";
import { string, toNumber } from "./globals/conversions.ts";
import {
  intlCanonicalLocales,
  intlCollatorCompare,
  intlDateTime,
  intlDateTimeParts,
  intlDisplayName,
  intlFractionDigits,
  intlHourCycle,
  intlListParts,
  intlLocaleInfo,
  intlNumber,
  intlNumberParts,
  intlPlural,
  intlPluralCategories,
  intlPluralRange,
  intlRelativeParts,
  intlSegment,
  type FormattedPart,
  type LocaleInfo,
  type RelativePart,
} from "tokamak:host";

// A part of a formatted range, and which end of the range it belongs to.
type RangePart = FormattedPart & { source: "shared" | "startRange" | "endRange" };

// A segment of a text, as a segmenter reports it.
interface SegmentData {
  segment: string;
  index: number;
  input: string;
  isWordLike?: boolean;
}

// Options as a formatter reads and keeps them, with the user's values.
type Options = Record<PropertyKey, unknown>;

// A formatter's canonical locales, as the JSON the host takes, and the locale
// formatting uses.
interface Locales {
  json: string;
  locale: string;
}

const dateFields = ["weekday", "era", "year", "month", "day", "dayPeriod", "hour", "minute", "second", "fractionalSecondDigits", "timeZoneName"];
const numberFields = ["minimumIntegerDigits", "minimumFractionDigits", "maximumFractionDigits", "minimumSignificantDigits", "maximumSignificantDigits", "useGrouping", "notation", "compactDisplay", "signDisplay", "roundingIncrement", "roundingMode", "roundingPriority", "trailingZeroDisplay", "currencySign", "currencyDisplay", "unitDisplay"];

function localeList(locales: unknown): string[] {
  if (locales === undefined) return [];
  if (typeof locales === "string" || (isObject(locales) && hostObjectKinds.get(locales) === "Intl.Locale")) return [String(locales)];
  if (locales === null) throw new TypeError("Invalid language tag");
  const source = objectOf(locales), values: string[] = [];
  const length = Math.min(Math.max(Math.trunc(toNumber(source.length)) || 0, 0), Number.MAX_SAFE_INTEGER);
  for (let i = 0; i < length; i++) {
    if (!(i in source)) continue;
    const value = source[i];
    if (typeof value !== "string" && (value === null || (typeof value !== "object" && typeof value !== "function"))) throw new TypeError("Invalid language tag");
    values.push(String(value));
  }
  return values;
}

function canonicalLocales(locales: unknown): string[] {
  return intlCanonicalLocales(localeList(locales));
}

function firstLocale(locales: unknown): string {
  return canonicalLocales(locales)[0] ?? "en-US";
}

function optionsObject(options: unknown): Options {
  if (options === undefined) return {};
  if (options === null) throw new TypeError("Options must not be null");
  return objectOf(options);
}

// The JSON the host takes for an object's internal options.
function optionsJson(options: object): string {
  return JSON.stringify(options, (_, value) => typeof value === "bigint" ? String(value) : value);
}

// Marks `object` as a host object and returns its canonical locales.
function initIntlObject(object: object, locales: unknown): Locales {
  markHostObject(object);
  const list = canonicalLocales(locales);
  return { json: JSON.stringify(list), locale: list[0] ?? "en-US" };
}

function localeInfo(locale: string): LocaleInfo {
  return intlLocaleInfo(locale);
}

function dateOptions(options: unknown): Options {
  const source = optionsObject(options);
  const result: Options = {};
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

function numberOptions(options: unknown): Options {
  const source = optionsObject(options);
  const result: Options = {};
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

// The plural rules' type, which ECMA-402 reads as a string of two values.
function pluralType(options: unknown): "cardinal" | "ordinal" {
  const type = optionsObject(options).type;
  const name = type === undefined ? "cardinal" : string(type);
  if (name !== "cardinal" && name !== "ordinal") throw new RangeError(`Value ${name} out of range for Intl.PluralRules options property type`);
  return name;
}

function listOptions(options: unknown): Options {
  const source = optionsObject(options);
  return { type: source.type ?? "conjunction", style: source.style ?? "long" };
}

function relativeOptions(options: unknown): Options {
  const source = optionsObject(options);
  return { numeric: source.numeric ?? "always", style: source.style ?? "long" };
}

/** The string a formatter's `format` returns: its parts' values in order. */
function joinParts(parts: { value: string }[]): string {
  return parts.map(part => part.value).join("");
}

function rangeParts(parts: FormattedPart[], source: RangePart["source"]): RangePart[] {
  return parts.map(part => ({ ...part, source }));
}

function joinRange(first: string, second: string): string {
  return first === second ? first : first + " – " + second;
}

function epochMilliseconds(value: unknown): number {
  const milliseconds = value === undefined ? Date.now() : toNumber(value);
  if (!Number.isFinite(milliseconds) || Math.abs(milliseconds) > 8_640_000_000_000_000) throw new RangeError("Invalid time value");
  return Math.trunc(milliseconds);
}

function dateResolvedOptions({ locale, json }: Locales, options: Options): Options {
  const info = localeInfo(locale);
  const result: Options = {
    locale,
    calendar: options.calendar ?? info.calendar,
    numberingSystem: options.numberingSystem ?? info.numberingSystem,
    timeZone: options.timeZone,
  };
  if (options.dateStyle !== undefined || options.timeStyle !== undefined) {
    if (options.dateStyle !== undefined) result.dateStyle = options.dateStyle;
    if (options.timeStyle !== undefined) {
      result.timeStyle = options.timeStyle;
      resolveHourCycle(result, json, options);
    }
    return result;
  }
  const hasDate = ["weekday", "era", "year", "month", "day"].some(field => options[field] !== undefined);
  const hasTime = ["dayPeriod", "hour", "minute", "second", "fractionalSecondDigits", "timeZoneName"].some(field => options[field] !== undefined);
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

function resolveHourCycle(result: Options, locales: string, options: Options): void {
  const hourCycle = options.hourCycle ?? (options.hour12 === undefined ? intlHourCycle(locales) : options.hour12 ? "h12" : "h23");
  result.hourCycle = hourCycle;
  result.hour12 = hourCycle === "h11" || hourCycle === "h12";
}

class DateTimeFormat {
  #locales: Locales;
  #options: Options;
  #optionsJson: string;
  #format: (value: unknown) => string;
  constructor(locales?: unknown, options?: unknown) {
    this.#locales = initIntlObject(this, locales);
    this.#options = dateOptions(options);
    this.#optionsJson = optionsJson(this.#options);
    this.#format = value => intlDateTime(epochMilliseconds(value), this.#locales.json, this.#optionsJson);
  }
  get format(): (value: unknown) => string { return this.#format; }
  formatToParts(value: unknown = new Date()): FormattedPart[] {
    return intlDateTimeParts(epochMilliseconds(value), this.#locales.json, this.#optionsJson);
  }
  formatRange(start: unknown, end: unknown): string { return joinRange(this.format(start), this.format(end)); }
  formatRangeToParts(start: unknown, end: unknown): RangePart[] {
    const first = this.formatToParts(start);
    const second = this.formatToParts(end);
    if (this.format(start) === this.format(end)) return rangeParts(first, "shared");
    return [...rangeParts(first, "startRange"), { type: "literal", value: " – ", source: "shared" }, ...rangeParts(second, "endRange")];
  }
  resolvedOptions(): Options { return dateResolvedOptions(this.#locales, this.#options); }
  static supportedLocalesOf(locales: unknown): string[] { return canonicalLocales(locales); }
}

function numberResolvedOptions(locale: string, options: Options, [minimumFractionDigits, maximumFractionDigits]: [number, number]): Options {
  const style = options.style;
  const result: Options = {
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
  #locales: Locales;
  #options: Options;
  #optionsJson: string;
  #fractionDigits: [number, number];
  #format: (value: unknown) => string;
  constructor(locales?: unknown, options?: unknown) {
    this.#locales = initIntlObject(this, locales);
    this.#options = numberOptions(options);
    this.#optionsJson = optionsJson(this.#options);
    this.#fractionDigits = intlFractionDigits(this.#optionsJson);
    this.#format = value => intlNumber(Number(value), this.#locales.json, this.#optionsJson);
  }
  get format(): (value: unknown) => string { return this.#format; }
  formatToParts(value: unknown): FormattedPart[] { return intlNumberParts(Number(value), this.#locales.json, this.#optionsJson); }
  formatRange(start: unknown, end: unknown): string { return joinRange(this.format(start), this.format(end)); }
  formatRangeToParts(start: unknown, end: unknown): RangePart[] {
    const first = this.format(start);
    const second = this.format(end);
    return first === second ? rangeParts(this.formatToParts(start), "shared") : [{ type: "literal", value: first, source: "startRange" }, { type: "literal", value: " – ", source: "shared" }, { type: "literal", value: second, source: "endRange" }];
  }
  resolvedOptions(): Options { return numberResolvedOptions(this.#locales.locale, this.#options, this.#fractionDigits); }
  static supportedLocalesOf(locales: unknown): string[] { return canonicalLocales(locales); }
}

class PluralRules {
  #locales: Locales;
  #type: "cardinal" | "ordinal";
  constructor(locales?: unknown, options?: unknown) {
    this.#locales = initIntlObject(this, locales);
    this.#type = pluralType(options);
  }
  select(value: unknown): string { return intlPlural(Number(value), this.#locales.json, this.#type); }
  selectRange(start: unknown, end: unknown): string { return intlPluralRange(Number(start), Number(end), this.#locales.json, this.#type); }
  resolvedOptions(): Options {
    return {
      locale: this.#locales.locale,
      type: this.#type,
      notation: "standard",
      minimumIntegerDigits: 1,
      minimumFractionDigits: 0,
      maximumFractionDigits: 3,
      pluralCategories: intlPluralCategories(this.#locales.json, this.#type),
      roundingIncrement: 1,
      roundingMode: "halfExpand",
      roundingPriority: "auto",
      trailingZeroDisplay: "auto",
    };
  }
  static supportedLocalesOf(locales: unknown): string[] { return canonicalLocales(locales); }
}

function localeTagWithOptions(tag: unknown, options: unknown): string {
  const base = String(tag);
  const source = optionsObject(options);
  const values: [string, string][] = [];
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
  #tag: string;
  #info: LocaleInfo;
  constructor(tag: unknown, options?: unknown) {
    markHostObject(this, "Intl.Locale");
    this.#tag = localeTagWithOptions(tag, options);
    this.#info = localeInfo(this.#tag);
  }
  get baseName(): string { return this.#info.baseName; }
  get language(): string { return this.#info.language; }
  get region(): string | undefined { return this.#info.region; }
  get script(): string | undefined { return this.#info.script; }
  get calendar(): string { return this.#info.calendar; }
  get caseFirst(): string | undefined { return this.#tag.match(/-kf-([a-z]+)/)?.[1] ?? undefined; }
  get collation(): string | undefined { return this.#tag.match(/-co-([a-z-]+)/)?.[1] ?? undefined; }
  get hourCycle(): string | undefined { return this.#info.hourCycle; }
  get numeric(): boolean { return /-kn(?:-|$)/.test(this.#tag); }
  toString(): string { return this.#info.string; }
  maximize(): Locale { return new Locale(this.#info.maximize); }
  minimize(): Locale { return new Locale(this.#info.minimize); }
  getHourCycles(): string[] { return [intlHourCycle(JSON.stringify([this.#tag]))]; }
  getTextInfo(): { direction: string } { return { direction: this.#info.direction }; }
  getWeekInfo(): { firstDay: number; weekend: number[] } { return { firstDay: this.#info.firstDay, weekend: this.#info.weekend }; }
}

// The strings of `list`, as ECMA-402's StringListFromIterable reads them,
// each converted with String.
function stringList(list: unknown): string[] {
  if (list === undefined) return [];
  if (!isIterable(list)) throw new TypeError(`${String(list)} is not iterable`);
  return Array.from(list, value => String(value));
}

class ListFormat {
  #locales: Locales;
  #options: Options;
  #optionsJson: string;
  constructor(locales?: unknown, options?: unknown) {
    this.#locales = initIntlObject(this, locales);
    this.#options = listOptions(options);
    this.#optionsJson = optionsJson(this.#options);
  }
  format(list: unknown): string { return joinParts(this.#parts(list)); }
  formatToParts(list: unknown): FormattedPart[] { return this.#parts(list); }
  #parts(list: unknown): FormattedPart[] { return intlListParts(stringList(list), this.#locales.json, this.#optionsJson); }
  resolvedOptions(): Options { return { locale: this.#locales.locale, ...this.#options }; }
  static supportedLocalesOf(locales: unknown): string[] { return canonicalLocales(locales); }
}

class RelativeTimeFormat {
  #locales: Locales;
  #options: Options;
  #optionsJson: string;
  constructor(locales?: unknown, options?: unknown) {
    this.#locales = initIntlObject(this, locales);
    this.#options = relativeOptions(options);
    this.#optionsJson = optionsJson(this.#options);
  }
  format(value: unknown, unit: unknown): string { return joinParts(this.#parts(value, unit)); }
  formatToParts(value: unknown, unit: unknown): RelativePart[] { return this.#parts(value, unit); }
  #parts(value: unknown, unit: unknown): RelativePart[] { return intlRelativeParts(Number(value), String(unit), this.#locales.json, this.#optionsJson); }
  resolvedOptions(): Options { return { locale: this.#locales.locale, ...this.#options, numberingSystem: localeInfo(this.#locales.locale).numberingSystem }; }
  static supportedLocalesOf(locales: unknown): string[] { return canonicalLocales(locales); }
}

class Collator {
  #locales: Locales;
  #options: Options;
  #compare: (left: unknown, right: unknown) => number;
  constructor(locales?: unknown, options?: unknown) {
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
    const json = optionsJson(this.#options);
    this.#compare = (left, right) => intlCollatorCompare(String(left), String(right), this.#locales.json, json);
  }
  get compare(): (left: unknown, right: unknown) => number { return this.#compare; }
  resolvedOptions(): Options { return { locale: this.#locales.locale, ...this.#options }; }
  static supportedLocalesOf(locales: unknown): string[] { return canonicalLocales(locales); }
}

class Segmenter {
  #locales: Locales;
  #granularity: "grapheme" | "word" | "sentence";
  constructor(locales?: unknown, options?: unknown) {
    this.#locales = initIntlObject(this, locales);
    const granularity = optionsObject(options).granularity ?? "grapheme";
    if (granularity !== "grapheme" && granularity !== "word" && granularity !== "sentence") throw new RangeError("Invalid granularity");
    this.#granularity = granularity;
  }
  segment(input: unknown): { containing(index: unknown): SegmentData | undefined; [Symbol.iterator](): Iterator<SegmentData> } {
    const source = String(input);
    const values = intlSegment(source, this.#granularity).map(({ segment, index, isWordLike }): SegmentData =>
      isWordLike === undefined ? { segment, index, input: source } : { segment, index, input: source, isWordLike });
    return {
      containing(index) {
        const position = Math.trunc(Number(index)) || 0;
        return values.find((value, valueIndex) => position >= value.index && position < (values[valueIndex + 1]?.index ?? source.length));
      },
      [Symbol.iterator]: function* () { yield* values; },
    };
  }
  resolvedOptions(): Options { return { locale: this.#locales.locale, granularity: this.#granularity }; }
  static supportedLocalesOf(locales: unknown): string[] { return canonicalLocales(locales); }
}

class DisplayNames {
  #locales: Locales;
  #options: Options;
  #optionsJson: string;
  constructor(locales: unknown, options: unknown) {
    this.#locales = initIntlObject(this, locales);
    const source = optionsObject(options);
    this.#options = {
      style: source.style ?? "long",
      type: source.type,
      fallback: source.fallback ?? "code",
      languageDisplay: source.languageDisplay ?? "dialect",
    };
    if (!this.#options.type) throw new TypeError("DisplayNames type is required");
    this.#optionsJson = optionsJson(this.#options);
  }
  of(value: unknown): string { return intlDisplayName(String(value), this.#locales.json, this.#optionsJson); }
  resolvedOptions(): Options { return { locale: this.#locales.locale, ...this.#options }; }
}

const supportedValues = new Map(Object.entries({
  calendar: ["buddhist", "chinese", "coptic", "dangi", "ethioaa", "ethiopic", "gregory", "hebrew", "indian", "islamic", "islamic-civil", "islamic-rgsa", "islamic-tbla", "islamic-umalqura", "iso8601", "japanese", "persian", "roc"],
  numberingSystem: ["adlm", "ahom", "arab", "arabext", "bali", "beng", "bhks", "brah", "cakm", "cham", "deva", "diak", "fullwide", "gong", "gonm", "gujr", "guru", "hanidec", "hmng", "hmnp", "java", "kali", "khmr", "knda", "lana", "lanatham", "laoo", "latn", "lepc", "limb", "mathbold", "mathdbl", "mathmono", "mathsanb", "mathsans", "mlym", "modi", "mong", "mroo", "mtei", "mymr", "mymrepka", "mymrpao", "nagm", "newa", "nkoo", "olck", "orya", "osma", "rohg", "saur", "segment", "shrd", "sind", "sinh", "sora", "sund", "takr", "talu", "taml", "tamldec", "telu", "thai", "tibt", "tirh", "tnsa", "vaii", "wara", "wcho"],
  timeZone: ["Africa/Abidjan", "Africa/Accra", "Africa/Addis_Ababa", "America/Adak", "America/Anchorage", "America/Argentina/Buenos_Aires", "America/Chicago", "America/Denver", "America/Los_Angeles", "America/New_York", "America/Phoenix", "America/Sao_Paulo", "America/Toronto", "Asia/Kolkata", "Asia/Tokyo", "Australia/Sydney", "Europe/Berlin", "Europe/London", "Europe/Paris", "Pacific/Auckland"],
}));

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
  getCanonicalLocales(locales: unknown): string[] { return canonicalLocales(locales); },
  supportedValuesOf(key: unknown): string[] {
    const values = supportedValues.get(string(key));
    if (values === undefined) throw new RangeError("Invalid key");
    return [...values];
  },
};

function dateLocaleString(value: unknown, locales: unknown, options: unknown, required: "date" | "time" | "any", defaults: "date" | "time" | "all"): string {
  const milliseconds: number = Reflect.apply(Date.prototype.getTime, value, []);
  if (Number.isNaN(milliseconds)) return "Invalid Date";
  const formatOptions: Options = Object.create(optionsObject(options));
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

export function installIntlGlobals(): void {
  Reflect.set(globalThis, "Intl", intl);
  Object.defineProperties(Date.prototype, {
    toLocaleString: { configurable: true, writable: true, value: function toLocaleString(this: unknown, locales: unknown, options: unknown) { return dateLocaleString(this, locales, options, "any", "all"); } },
    toLocaleDateString: { configurable: true, writable: true, value: function toLocaleDateString(this: unknown, locales: unknown, options: unknown) { return dateLocaleString(this, locales, options, "date", "date"); } },
    toLocaleTimeString: { configurable: true, writable: true, value: function toLocaleTimeString(this: unknown, locales: unknown, options: unknown) { return dateLocaleString(this, locales, options, "time", "time"); } },
  });
  Object.defineProperty(Number.prototype, "toLocaleString", { configurable: true, writable: true,
    value: function toLocaleString(this: unknown, locales: unknown, options: unknown) { return new NumberFormat(locales, options).format(Reflect.apply(Number.prototype.valueOf, this, [])); } });
  Object.defineProperty(String.prototype, "localeCompare", { configurable: true, writable: true,
    value: function localeCompare(this: unknown, other: unknown, locales: unknown, options: unknown) {
      if (this == null) throw new TypeError("String.prototype.localeCompare called on null or undefined");
      return new Collator(locales, options).compare(string(this), string(other));
    } });
}

export { intl };
