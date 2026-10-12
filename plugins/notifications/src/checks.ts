/** Checks of the values the plugin's methods take, which throw `TypeError` naming the value. */
export type Check = (value: unknown, path: string) => void;

/** Checks each field `checks` names that the object `value` has, or that `required` lists. */
export function fields(
  value: Record<string, unknown>,
  path: string,
  checks: Readonly<Record<string, Check>>,
  required: readonly string[] = [],
): void {
  for (const [key, check] of Object.entries(checks)) {
    if (value[key] !== undefined || required.includes(key)) {
      check(value[key], path === "" ? key : `${path}.${key}`);
    }
  }
}

/** Checks that `value` is an object, then its fields as `fields` does. */
export function object(
  checks: Readonly<Record<string, Check>>,
  required: readonly string[] = [],
): Check {
  return (value, path) => {
    record(value, path);
    fields(value, path, checks, required);
  };
}

export function record(value: unknown, path: string): asserts value is Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new TypeError(`${path} must be an object`);
  }
}

export function text(value: unknown, path: string): void {
  if (typeof value !== "string") throw new TypeError(`${path} must be a string`);
}

export function nonEmptyText(value: unknown, path: string): void {
  if (typeof value !== "string" || value === "") {
    throw new TypeError(`${path} must be a non-empty string`);
  }
}

/** An array of entries `check` accepts. */
export function array(check: Check): Check {
  return (value, path) => {
    if (!Array.isArray(value)) throw new TypeError(`${path} must be an array`);
    for (const [index, entry] of (value as unknown[]).entries()) {
      check(entry, `${path}[${String(index)}]`);
    }
  };
}

export function flag(value: unknown, path: string): void {
  if (typeof value !== "boolean") throw new TypeError(`${path} must be a boolean`);
}

/** A whole number of at least 0. */
export function count(value: unknown, path: string): void {
  if (!Number.isInteger(value) || (value as number) < 0) {
    throw new TypeError(`${path} must be a whole number of at least 0`);
  }
}

/** Milliseconds since the epoch. */
export function time(value: unknown, path: string): void {
  if (typeof value !== "number" || !Number.isFinite(value)) {
    throw new TypeError(`${path} must be a number of milliseconds since the epoch`);
  }
}

/** A vibration pattern: whole milliseconds, alternating between off and on or on and off. */
export function pattern(value: unknown, path: string): void {
  if (!Array.isArray(value) || !value.every((step) => Number.isInteger(step) && step >= 0)) {
    throw new TypeError(`${path} must be an array of whole milliseconds`);
  }
}

/** A colour as `#rrggbb`. */
export function colour(value: unknown, path: string): void {
  if (!(typeof value === "string" && /^#[0-9a-f]{6}$/i.test(value))) {
    throw new TypeError(`${path} must be a colour such as "#00ff00"`);
  }
}

export function oneOf(...allowed: readonly string[]): Check {
  return (value, path) => {
    if (!allowed.includes(value as string)) {
      throw new TypeError(`${path} must be one of ${allowed.map((name) => `"${name}"`).join(", ")}`);
    }
  };
}

/** The origin a path resolves against to check that it stays on the app's own. */
const APP_ORIGIN = "https://app.invalid";

/**
 * An `https` URL, or a path the app serves: one that starts with `/` and that browsers do not read
 * as another host's, as they read `//host` and `/\host`. Neither may hold whitespace or control
 * characters, which browsers remove or encode and native URL parsers keep, encode or reject.
 */
export function source(value: unknown, path: string): void {
  if (
    typeof value === "string" &&
    !/[\s\p{Cc}]/u.test(value) &&
    (value.startsWith("/") ? isAppPath(value) : isHttps(value))
  ) {
    return;
  }
  throw new TypeError(
    `${path} must be an https URL or a path that starts with /, with any spaces percent-encoded`,
  );
}

/** A number from 0 to 1. */
export function fraction(value: unknown, path: string): void {
  if (!(typeof value === "number" && value >= 0 && value <= 1)) {
    throw new TypeError(`${path} must be a number from 0 to 1`);
  }
}

function isAppPath(value: string): boolean {
  return parse(value, APP_ORIGIN)?.origin === APP_ORIGIN;
}

/** Whether `value` is an `https` URL that names its host after `//`, as native URL parsers require. */
function isHttps(value: string): boolean {
  return /^https:\/\/[^/\\]/i.test(value) && parse(value) !== undefined;
}

function parse(value: string, base?: string): URL | undefined {
  try {
    return new URL(value, base);
  } catch {
    return undefined;
  }
}
