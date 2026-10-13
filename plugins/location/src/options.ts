import {
  atLeastZero,
  between,
  flag,
  nonEmptyText,
  object,
  oneOf,
  positive,
  text,
  wholeAtLeastZero,
} from "./checks.js";
import type { Region } from "./types.js";

/** Throws `TypeError` unless `config` is a configuration `setup` takes. */
export const validateSetup = object({ significantChanges: flag, visits: flag });

/** Throws `TypeError` unless `options` are options `requestPermission` takes. */
export const validatePermissionRequest = object({
  background: flag,
  precision: oneOf("precise", "approximate"),
  purpose: nonEmptyText,
});

/** Throws `TypeError` unless `options` are options `startUpdates` takes. */
export const validateUpdateOptions = object({
  activity: oneOf("default", "automotiveNavigation", "otherNavigation", "fitness", "airborne", "maritime"),
  accuracy: oneOf("high", "balanced", "low"),
  distance: atLeastZero,
  interval: wholeAtLeastZero,
  foregroundService: object(
    { title: nonEmptyText, body: text, channel: nonEmptyText },
    ["title", "body", "channel"],
  ),
});

const REGION = object(
  {
    id: nonEmptyText,
    latitude: between(-90, 90),
    longitude: between(-180, 180),
    radius: positive,
    onEntry: flag,
    onExit: flag,
  },
  ["id", "latitude", "longitude", "radius"],
);

/** `region`, with `onEntry` and `onExit` given. Throws `TypeError` unless it is a region to monitor. */
export function monitoredRegion(region: Region): Required<Region> {
  REGION(region, "region");
  const { id, latitude, longitude, radius, onEntry = true, onExit = true } = region;
  if (!onEntry && !onExit) throw new TypeError("region must report entry, exit or both");
  return { id, latitude, longitude, radius, onEntry, onExit };
}
