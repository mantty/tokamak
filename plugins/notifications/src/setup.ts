import type { Importance } from "./index.js";

/** The id of the Android channel `defaultChannel` describes. */
const DEFAULT_CHANNEL = "tokamak.notifications";

const IMPORTANCES: readonly Importance[] = ["none", "min", "low", "default", "high"];

/** Throws `TypeError`, naming the first invalid value, unless `value` is a valid configuration. */
export function validateSetup(value: unknown): void {
  if (!isRecord(value)) throw new TypeError("setup takes a configuration object");
  list(value.categories, "categories", (category, path) => {
    record(category, path);
    text(category.id, `${path}.id`, true);
    list(category.actions, `${path}.actions`, (action, actionPath) => {
      record(action, actionPath);
      text(action.id, `${actionPath}.id`, true);
      text(action.title, `${actionPath}.title`);
      if (action.input !== undefined) {
        record(action.input, `${actionPath}.input`);
        optionalText(action.input.button, `${actionPath}.input.button`);
        optionalText(action.input.placeholder, `${actionPath}.input.placeholder`);
      }
      for (const key of ["foreground", "destructive", "authenticationRequired"]) {
        optionalFlag(action[key], `${actionPath}.${key}`);
      }
      optionalText(action.systemImage, `${actionPath}.systemImage`);
    }, true);
    for (const key of ["hiddenPreviewsBodyPlaceholder", "summaryFormat"]) {
      optionalText(category[key], `${path}.${key}`);
    }
    for (const key of [
      "hiddenPreviewsShowTitle",
      "hiddenPreviewsShowSubtitle",
      "customDismissAction",
      "allowInCarPlay",
    ]) {
      optionalFlag(category[key], `${path}.${key}`);
    }
  });
  const shown = value.showInForeground;
  if (isRecord(shown)) {
    for (const key of ["banner", "list", "sound", "badge"]) {
      optionalFlag(shown[key], `showInForeground.${key}`);
    }
  } else {
    optionalFlag(shown, "showInForeground");
  }
  const groups = new Set<string>();
  list(value.channelGroups, "channelGroups", (group, path) => {
    record(group, path);
    text(group.id, `${path}.id`, true);
    text(group.name, `${path}.name`);
    optionalText(group.description, `${path}.description`);
    groups.add(group.id as string);
  });
  if (value.defaultChannel !== undefined) channel(value.defaultChannel, "defaultChannel", groups);
  list(value.channels, "channels", (entry, path) => {
    channel(entry, path, groups);
    text(entry.id, `${path}.id`, true);
    if (entry.id === DEFAULT_CHANNEL) {
      throw new TypeError(`${path}.id is the default channel's; describe it with defaultChannel`);
    }
  });
}

function channel(
  value: unknown,
  path: string,
  groups: ReadonlySet<string>,
): asserts value is Record<string, unknown> {
  record(value, path);
  text(value.name, `${path}.name`);
  optionalText(value.description, `${path}.description`);
  if (value.group !== undefined && !groups.has(value.group as string)) {
    throw new TypeError(`${path}.group must be a channelGroups id`);
  }
  oneOf(value.importance, IMPORTANCES, `${path}.importance`);
  optionalText(value.sound, `${path}.sound`);
  const vibration = value.vibration;
  if (Array.isArray(vibration)) {
    if (vibration.length === 0 || !vibration.every((step) => Number.isInteger(step) && step >= 0)) {
      throw new TypeError(`${path}.vibration must be a boolean or a non-empty array of milliseconds`);
    }
  } else {
    optionalFlag(vibration, `${path}.vibration`);
  }
  const lights = value.lights;
  const colour = /^#[0-9a-f]{6}$/i;
  if (lights !== undefined && lights !== false && !(typeof lights === "string" && colour.test(lights))) {
    throw new TypeError(`${path}.lights must be false or a colour such as "#00ff00"`);
  }
  optionalFlag(value.showBadge, `${path}.showBadge`);
  optionalFlag(value.bypassDnd, `${path}.bypassDnd`);
}

/** Validates each entry of an optional array, whose `id`s must differ. */
function list(
  value: unknown,
  path: string,
  validate: (entry: Record<string, unknown>, path: string) => void,
  required = false,
): void {
  if (value === undefined && !required) return;
  if (!Array.isArray(value)) throw new TypeError(`${path} must be an array`);
  const ids = new Set<unknown>();
  for (const [index, entry] of (value as Record<string, unknown>[]).entries()) {
    const entryPath = `${path}[${String(index)}]`;
    validate(entry, entryPath);
    if (ids.has(entry.id)) throw new TypeError(`${entryPath}.id repeats an earlier id`);
    ids.add(entry.id);
  }
}

function record(value: unknown, path: string): asserts value is Record<string, unknown> {
  if (!isRecord(value)) throw new TypeError(`${path} must be an object`);
}

function text(value: unknown, path: string, nonEmpty = false): void {
  if (typeof value !== "string" || (nonEmpty && value === "")) {
    throw new TypeError(`${path} must be a ${nonEmpty ? "non-empty " : ""}string`);
  }
}

function optionalText(value: unknown, path: string): void {
  if (value !== undefined) text(value, path);
}

function optionalFlag(value: unknown, path: string): void {
  if (value !== undefined && typeof value !== "boolean") {
    throw new TypeError(`${path} must be a boolean`);
  }
}

function oneOf(value: unknown, allowed: readonly string[], path: string): void {
  if (value !== undefined && !allowed.includes(value as string)) {
    throw new TypeError(`${path} must be one of ${allowed.map((name) => `"${name}"`).join(", ")}`);
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
