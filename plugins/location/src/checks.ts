/** Checks of the values the plugin's methods take, which throw `TypeError` naming the value. */
export type Check = (value: unknown, path: string) => void;

/** Checks that `value` is an object, then each field `checks` names that it has or `required` lists. */
export function object(checks: Readonly<Record<string, Check>>, required: readonly string[] = []): Check {
  return (value, path) => {
    if (typeof value !== "object" || value === null || Array.isArray(value)) {
      throw new TypeError(`${path} must be an object`);
    }
    for (const [key, check] of Object.entries(checks)) {
      const field = (value as Record<string, unknown>)[key];
      if (field !== undefined || required.includes(key)) check(field, `${path}.${key}`);
    }
  };
}

export function flag(value: unknown, path: string): void {
  if (typeof value !== "boolean") throw new TypeError(`${path} must be a boolean`);
}

export function text(value: unknown, path: string): void {
  if (typeof value !== "string") throw new TypeError(`${path} must be a string`);
}

export function nonEmptyText(value: unknown, path: string): void {
  if (typeof value !== "string" || value === "") throw new TypeError(`${path} must be a non-empty string`);
}

export function oneOf(...allowed: readonly string[]): Check {
  return (value, path) => {
    if (!allowed.includes(value as string)) {
      throw new TypeError(`${path} must be one of ${allowed.map((name) => `"${name}"`).join(", ")}`);
    }
  };
}

/** A finite number of at least 0. */
export function atLeastZero(value: unknown, path: string): void {
  if (!(typeof value === "number" && Number.isFinite(value) && value >= 0)) {
    throw new TypeError(`${path} must be a finite number of at least 0`);
  }
}

/** A whole number of at least 0. */
export function wholeAtLeastZero(value: unknown, path: string): void {
  if (!(Number.isSafeInteger(value) && (value as number) >= 0)) {
    throw new TypeError(`${path} must be a whole number of at least 0`);
  }
}
