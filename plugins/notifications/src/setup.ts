import {
  type Check,
  array,
  colour,
  fields,
  fileName,
  flag,
  nonEmptyText,
  object,
  oneOf,
  pattern,
  record,
  source,
  text,
} from "./checks.js";

/** The id of the Android channel `defaultChannel` describes. */
const DEFAULT_CHANNEL = "tokamak.notifications";

const ACTION = object(
  {
    id: nonEmptyText,
    title: text,
    input: object({ button: text, placeholder: text }),
    foreground: flag,
    destructive: flag,
    authenticationRequired: flag,
    systemImage: text,
    icon: source,
  },
  ["id", "title"],
);

const CATEGORY = object(
  {
    id: nonEmptyText,
    actions: list(ACTION),
    hiddenPreviewsBodyPlaceholder: text,
    summaryFormat: text,
    hiddenPreviewsShowTitle: flag,
    hiddenPreviewsShowSubtitle: flag,
    customDismissAction: flag,
    allowInCarPlay: flag,
  },
  ["id", "actions"],
);

const CHANNEL_GROUP = object({ id: nonEmptyText, name: text, description: text }, ["id", "name"]);

/** Throws `TypeError`, naming the first invalid value, unless `value` is a valid configuration. */
export function validateSetup(value: unknown): void {
  record(value, "setup's configuration");
  const groups = new Set(
    Array.isArray(value.channelGroups)
      ? (value.channelGroups as unknown[]).map((group) => (group as { id?: unknown } | null)?.id)
      : [],
  );
  fields(value, "", {
    categories: list(CATEGORY),
    showInForeground: (shown, path) => {
      if (typeof shown === "boolean") return;
      if (typeof shown !== "object" || shown === null || Array.isArray(shown)) {
        throw new TypeError(`${path} must be a boolean or an object`);
      }
      object({ banner: flag, list: flag, sound: flag, badge: flag })(shown, path);
    },
    channelGroups: list(CHANNEL_GROUP),
    defaultChannel: channel(groups),
    channels: list((entry, path) => {
      channel(groups)(entry, path);
      fields(entry as Record<string, unknown>, path, { id: nonEmptyText }, ["id"]);
      if ((entry as { id: unknown }).id === DEFAULT_CHANNEL) {
        throw new TypeError(`${path}.id is the default channel's; describe it with defaultChannel`);
      }
    }),
  });
}

/** An Android channel's settings, whose `group` must be one of `groups`. */
function channel(groups: ReadonlySet<unknown>): Check {
  return object(
    {
      name: text,
      description: text,
      group: (group, path) => {
        if (!groups.has(group)) throw new TypeError(`${path} must be a channelGroups id`);
      },
      importance: oneOf("none", "min", "low", "default", "high"),
      sound: (sound, path) => {
        if (sound !== "default" && sound !== "none") fileName(sound, path);
      },
      vibration: (vibration, path) => {
        if (typeof vibration === "boolean") return;
        if (!Array.isArray(vibration) || vibration.length === 0) {
          throw new TypeError(`${path} must be a boolean or a non-empty array of milliseconds`);
        }
        pattern(vibration, path);
      },
      lights: (lights, path) => {
        if (lights === false) return;
        if (typeof lights !== "string") {
          throw new TypeError(`${path} must be false or a colour such as "#00ff00"`);
        }
        colour(lights, path);
      },
      showBadge: flag,
      bypassDnd: flag,
    },
    ["name"],
  );
}

/** An array of entries `check` accepts, whose `id`s differ. */
function list(check: Check): Check {
  return (value, path) => {
    array(check)(value, path);
    const ids = new Set<unknown>();
    for (const [index, { id }] of (value as { id: unknown }[]).entries()) {
      if (ids.has(id)) throw new TypeError(`${path}[${String(index)}].id repeats an earlier id`);
      ids.add(id);
    }
  };
}
