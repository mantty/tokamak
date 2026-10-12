import { between, count, fields, flag, object, time, whole } from "./checks.js";

const TRIGGERS = ["at", "every", "on", "region"] as const;

/** The shortest repeating interval Apple allows. */
const MIN_INTERVAL_SECONDS = 60;

const DATE_PARTS = {
  year: whole(1, 9999),
  month: whole(1, 12),
  day: whole(1, 31),
  weekday: whole(1, 7),
  hour: whole(0, 23),
  minute: whole(0, 59),
  second: whole(0, 59),
};

const REGION = object(
  {
    latitude: between(-90, 90),
    longitude: between(-180, 180),
    radius: (value, path) => {
      if (!(typeof value === "number" && value > 0 && Number.isFinite(value))) {
        throw new TypeError(`${path} must be a number of metres above 0`);
      }
    },
    onEntry: flag,
    onExit: flag,
  },
  ["latitude", "longitude", "radius"],
);

/** Throws `TypeError` unless the scheduled notification `value` has one valid trigger. */
export function validateTrigger(value: Record<string, unknown>): void {
  const given = TRIGGERS.filter((key) => value[key] !== undefined);
  if (given.length !== 1) {
    throw new TypeError("a scheduled notification must have one of at, every, on or region");
  }
  const [trigger] = given;
  fields(value, "", { at: time, every: interval, on: dateMatch, region, repeats: flag, exact: flag });
  if (value.repeats !== undefined && (trigger === "at" || trigger === "every")) {
    throw new TypeError("repeats applies only to on and region triggers");
  }
  if (value.exact !== undefined && trigger === "region") {
    throw new TypeError("exact does not apply to a region trigger");
  }
}

function interval(value: unknown, path: string): void {
  object({ seconds: count, minutes: count, hours: count, days: count })(value, path);
  const { seconds = 0, minutes = 0, hours = 0, days = 0 } = value as Record<string, number | undefined>;
  if (seconds + 60 * (minutes + 60 * (hours + 24 * days)) < MIN_INTERVAL_SECONDS) {
    throw new TypeError(`${path} must be at least ${String(MIN_INTERVAL_SECONDS)} seconds`);
  }
}

function dateMatch(value: unknown, path: string): void {
  object(DATE_PARTS)(value, path);
  if (!Object.keys(DATE_PARTS).some((part) => (value as Record<string, unknown>)[part] !== undefined)) {
    throw new TypeError(`${path} must name at least one of ${Object.keys(DATE_PARTS).join(", ")}`);
  }
}

function region(value: unknown, path: string): void {
  REGION(value, path);
  const { onEntry, onExit } = value as { onEntry?: boolean; onExit?: boolean };
  if (onEntry === false && onExit !== true) {
    throw new TypeError(`${path} must fire on entry, exit or both`);
  }
}
