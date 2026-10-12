import assert from "node:assert/strict";
import { test } from "node:test";

import { validateContent } from "../src/content.js";
import { validateBadge, validateOpenSettings, validatePermissionOptions, validateTopic } from "../src/options.js";
import { validateSetup } from "../src/setup.js";

const notification = { id: "n1", title: "Hi" };
const messaging = { type: "messaging", user: "Me", messages: [] };
const action = { id: "a", title: "A" };
const channel = { id: "c", name: "C" };
const category = (extra: object) => ({ categories: [{ id: "c", actions: [], ...extra }] });
const actionWith = (extra: object) => ({ categories: [{ id: "c", actions: [{ ...action, ...extra }] }] });

const INVALID_NOTIFICATIONS: [unknown, string][] = [
  [null, "notification"],
  [{ title: "Hi" }, "id"],
  [{ ...notification, id: "" }, "id"],
  [{ id: "n1" }, "title"],
  [{ ...notification, title: 1 }, "title"],
  [{ ...notification, body: 1 }, "body"],
  [{ ...notification, data: [] }, "data"],
  [{ ...notification, data: null }, "data"],
  [{ ...notification, category: "" }, "category"],
  [{ ...notification, channel: "" }, "channel"],
  [{ ...notification, group: "" }, "group"],
  [{ ...notification, badge: -1 }, "badge"],
  [{ ...notification, badge: 1.5 }, "badge"],
  [{ ...notification, silent: "yes" }, "silent"],
  [{ ...notification, timestamp: Number.NaN }, "timestamp"],
  [{ ...notification, subtitle: 1 }, "subtitle"],
  [{ ...notification, interruptionLevel: "urgent" }, "interruptionLevel"],
  [{ ...notification, relevanceScore: 2 }, "relevanceScore"],
  [{ ...notification, relevanceScore: -0.1 }, "relevanceScore"],
  [{ ...notification, relevanceScore: "0.5" }, "relevanceScore"],
  [{ ...notification, subText: 1 }, "subText"],
  [{ ...notification, color: "#00ff0080" }, "color"],
  [{ ...notification, style: { type: "bigVideo" } }, "style.type"],
  [{ ...notification, style: { type: "bigPicture" } }, "image"],
  [{ ...notification, image: "/i.png", style: { type: "bigPicture", showWhenCollapsed: "yes" } }, "style.showWhenCollapsed"],
  [{ ...notification, image: "http://example.com/i.png" }, "image"],
  [{ ...notification, image: "//example.com/i.png" }, "image"],
  [{ ...notification, image: "i.png" }, "image"],
  [{ ...notification, image: "/\\example.com/i.png" }, "image"],
  [{ ...notification, image: "/\t/example.com/i.png" }, "image"],
  [{ ...notification, image: "https:example.com/i.png" }, "image"],
  [{ ...notification, image: "https:///example.com/i.png" }, "image"],
  [{ ...notification, image: "https://\\example.com/i.png" }, "image"],
  [{ ...notification, image: "https://exa mple.com/i.png" }, "image"],
  [{ ...notification, image: "https://example.com:99999/i.png" }, "image"],
  [{ ...notification, image: "https://\texample.com/i.png" }, "image"],
  [{ ...notification, image: "https://example.com/i.png " }, "image"],
  [{ ...notification, image: "/a\n/b.png" }, "image"],
  [{ ...notification, image: "/my photo.png" }, "image"],
  [{ ...notification, icon: "http://example.com/i.png" }, "icon"],
  [{ ...notification, sound: "chime.caf" }, "sound"],
  [{ ...notification, sound: {} }, "sound.name"],
  [{ ...notification, sound: { name: "chime" } }, "sound.name"],
  [{ ...notification, sound: { name: "sounds/chime.caf" } }, "sound.name"],
  [{ ...notification, sound: { name: "chime.caf", volume: 0.5 } }, "sound.volume"],
  [{ ...notification, sound: { name: "chime.caf", critical: true, volume: 2 } }, "sound.volume"],
  [{ ...notification, silent: true, sound: "default" }, "sound"],
  [{ ...notification, settingsText: "" }, "settingsText"],
  [{ ...notification, icon: 1 }, "icon"],
  [{ ...notification, attachments: {} }, "attachments"],
  [{ ...notification, attachments: [{}] }, "attachments[0].url"],
  [{ ...notification, attachments: [{ url: "/a.png", hideThumbnail: "yes" }] }, "attachments[0].hideThumbnail"],
  [
    { ...notification, attachments: [{ url: "/a.png", thumbnailClippingRect: { x: 2, y: 0, width: 1, height: 1 } }] },
    "attachments[0].thumbnailClippingRect.x",
  ],
  [{ ...notification, attachments: [{ url: "/a.png", thumbnailClippingRect: { x: 0 } }] }, "attachments[0].thumbnailClippingRect.y"],
  [
    { ...notification, attachments: [{ url: "/a.png", thumbnailClippingRect: { x: -0.1, y: 0, width: 1, height: 1 } }] },
    "attachments[0].thumbnailClippingRect.x",
  ],
  [
    { ...notification, attachments: [{ url: "/a.png", thumbnailClippingRect: { x: Number.NaN, y: 0, width: 1, height: 1 } }] },
    "attachments[0].thumbnailClippingRect.x",
  ],
  [
    { ...notification, attachments: [{ url: "/a.png", thumbnailClippingRect: { x: 0.8, y: 0, width: 0.5, height: 1 } }] },
    "attachments[0].thumbnailClippingRect",
  ],
  [
    { ...notification, attachments: [{ url: "/a.png", thumbnailClippingRect: { x: 0, y: 0, width: 0, height: 1 } }] },
    "attachments[0].thumbnailClippingRect",
  ],
  [
    { ...notification, attachments: [{ url: "/a.png", thumbnailClippingRect: { x: 0, y: 0, width: 1, height: 0 } }] },
    "attachments[0].thumbnailClippingRect",
  ],
  [
    { ...notification, attachments: [{ url: "/a.png", thumbnailClippingRect: { x: 0, y: 0.8, width: 1, height: 0.5 } }] },
    "attachments[0].thumbnailClippingRect",
  ],
  [{ ...notification, attachments: [{ url: "/a.mp4", thumbnailTime: -1 }] }, "attachments[0].thumbnailTime"],
  [{ ...notification, attachments: [{ url: "/a.mp4", thumbnailTime: "2" }] }, "attachments[0].thumbnailTime"],
  [
    { ...notification, attachments: [{ url: "/a.mp4", thumbnailTime: Number.POSITIVE_INFINITY }] },
    "attachments[0].thumbnailTime",
  ],
  [{ ...notification, attachments: [{ url: "/a.png" }, { url: "http://example.com/a.png" }] }, "attachments[1].url"],
  [{ ...notification, style: { type: "toString" } }, "style.type"],
  [{ ...notification, style: { type: "bigText" } }, "style.text"],
  [{ ...notification, style: { type: "bigText", text: "t", title: 1 } }, "style.title"],
  [{ ...notification, style: { type: "inbox" } }, "style.lines"],
  [{ ...notification, style: { type: "inbox", lines: [1] } }, "style.lines[0]"],
  [{ ...notification, style: { type: "messaging", messages: [] } }, "style.user"],
  [{ ...notification, style: { ...messaging, user: 1 } }, "style.user"],
  [{ ...notification, style: { type: "messaging", user: "Me" } }, "style.messages"],
  [{ ...notification, style: { ...messaging, messages: {} } }, "style.messages"],
  [{ ...notification, style: { ...messaging, conversationTitle: 1 } }, "style.conversationTitle"],
  [{ ...notification, style: { ...messaging, messages: [{ at: 0 }] } }, "style.messages[0].text"],
  [{ ...notification, style: { ...messaging, messages: [{ text: "Hi" }] } }, "style.messages[0].at"],
  [{ ...notification, style: { ...messaging, messages: [{ text: "Hi", at: 0, sender: 1 }] } }, "style.messages[0].sender"],
  [{ ...notification, progress: { current: 6, max: 5 } }, "progress.current"],
  [{ ...notification, progress: { current: 1 } }, "progress.max"],
  [{ ...notification, progress: { current: -1, max: 5 } }, "progress.current"],
  [{ ...notification, progress: { indeterminate: 1 } }, "progress.current"],
  [{ ...notification, ongoing: 1 }, "ongoing"],
  [{ ...notification, autoCancel: 1 }, "autoCancel"],
  [{ ...notification, onlyAlertOnce: 1 }, "onlyAlertOnce"],
  [{ ...notification, timeoutAfter: -1 }, "timeoutAfter"],
  [{ ...notification, chronometer: {} }, "chronometer.countDown"],
  [{ ...notification, chronometer: { countDown: "yes" } }, "chronometer.countDown"],
  [{ ...notification, visibility: "hidden" }, "visibility"],
  [{ ...notification, publicVersion: { body: "No title" } }, "publicVersion.title"],
  [{ ...notification, publicVersion: { title: "t", body: 1 } }, "publicVersion.body"],
  [{ ...notification, sortKey: 1 }, "sortKey"],
  [{ ...notification, androidCategory: "msg" }, "androidCategory"],
  [{ ...notification, requireInteraction: 1 }, "requireInteraction"],
  [{ ...notification, vibrate: [-1] }, "vibrate"],
  [{ ...notification, silent: true, vibrate: [100] }, "vibrate"],
  [{ ...notification, dir: "up" }, "dir"],
  [{ ...notification, lang: 1 }, "lang"],
  [{ ...notification, badgeImage: 1 }, "badgeImage"],
];

const INVALID_SETUPS: [unknown, string][] = [
  [null, "setup's configuration"],
  ["config", "setup's configuration"],
  [[], "setup's configuration"],
  [{ categories: [{ id: "", actions: [] }] }, "categories[0].id"],
  [{ categories: [{ actions: [] }] }, "categories[0].id"],
  [{ categories: [{ id: "a" }] }, "categories[0].actions"],
  [{ categories: [{ id: "a", actions: [action, action] }] }, "categories[0].actions[1].id"],
  [{ categories: [{ id: "a", actions: [] }, { id: "a", actions: [] }] }, "categories[1].id"],
  [actionWith({ id: undefined }), "categories[0].actions[0].id"],
  [actionWith({ id: "" }), "categories[0].actions[0].id"],
  [actionWith({ title: undefined }), "categories[0].actions[0].title"],
  [actionWith({ foreground: "yes" }), "categories[0].actions[0].foreground"],
  [actionWith({ input: "yes" }), "categories[0].actions[0].input"],
  [actionWith({ input: { button: 1 } }), "categories[0].actions[0].input.button"],
  [actionWith({ input: { placeholder: 1 } }), "categories[0].actions[0].input.placeholder"],
  [actionWith({ destructive: "yes" }), "categories[0].actions[0].destructive"],
  [actionWith({ authenticationRequired: "yes" }), "categories[0].actions[0].authenticationRequired"],
  [actionWith({ systemImage: 1 }), "categories[0].actions[0].systemImage"],
  [actionWith({ icon: "ftp://example.com/i.png" }), "categories[0].actions[0].icon"],
  [category({ hiddenPreviewsBodyPlaceholder: 1 }), "categories[0].hiddenPreviewsBodyPlaceholder"],
  [category({ summaryFormat: 1 }), "categories[0].summaryFormat"],
  [category({ hiddenPreviewsShowTitle: "yes" }), "categories[0].hiddenPreviewsShowTitle"],
  [category({ hiddenPreviewsShowSubtitle: "yes" }), "categories[0].hiddenPreviewsShowSubtitle"],
  [category({ customDismissAction: "yes" }), "categories[0].customDismissAction"],
  [category({ allowInCarPlay: "yes" }), "categories[0].allowInCarPlay"],
  [{ showInForeground: "yes" }, "showInForeground"],
  [{ showInForeground: { banner: 1 } }, "showInForeground.banner"],
  [{ showInForeground: { list: 1 } }, "showInForeground.list"],
  [{ showInForeground: { sound: 1 } }, "showInForeground.sound"],
  [{ showInForeground: { badge: 1 } }, "showInForeground.badge"],
  [{ channelGroups: [{ id: "g", name: "G" }, { id: "g", name: "G" }] }, "channelGroups[1].id"],
  [{ channelGroups: [{ name: "G" }] }, "channelGroups[0].id"],
  [{ channelGroups: [{ id: "", name: "G" }] }, "channelGroups[0].id"],
  [{ channelGroups: [{ id: "g" }] }, "channelGroups[0].name"],
  [{ channelGroups: [{ id: "g", name: "G", description: 1 }] }, "channelGroups[0].description"],
  [{ channels: [channel, channel] }, "channels[1].id"],
  [{ channels: [{ name: "C" }] }, "channels[0].id"],
  [{ channels: [{ ...channel, id: "" }] }, "channels[0].id"],
  [{ channels: [{ id: "c" }] }, "channels[0].name"],
  [{ channels: [{ ...channel, description: 1 }] }, "channels[0].description"],
  [{ channels: [{ ...channel, group: "missing" }] }, "channels[0].group"],
  [{ channels: [{ ...channel, id: "tokamak.notifications" }] }, "channels[0].id"],
  [{ channels: [{ ...channel, importance: "urgent" }] }, "channels[0].importance"],
  [{ channels: [{ ...channel, sound: 1 }] }, "channels[0].sound"],
  [{ channels: [{ ...channel, vibration: "yes" }] }, "channels[0].vibration"],
  [{ channels: [{ ...channel, vibration: [] }] }, "channels[0].vibration"],
  [{ channels: [{ ...channel, vibration: [-1] }] }, "channels[0].vibration"],
  [{ channels: [{ ...channel, vibration: [100, 0.5] }] }, "channels[0].vibration"],
  // Android reads eight digits as alpha first, CSS as alpha last.
  [{ channels: [{ ...channel, lights: "#00ff0080" }] }, "channels[0].lights"],
  [{ channels: [{ ...channel, lights: true }] }, "channels[0].lights"],
  [{ channels: [{ ...channel, showBadge: "yes" }] }, "channels[0].showBadge"],
  [{ channels: [{ ...channel, bypassDnd: "yes" }] }, "channels[0].bypassDnd"],
  [{ defaultChannel: { description: "No name" } }, "defaultChannel.name"],
  [{ defaultChannel: { name: "General", sound: "chime" } }, "defaultChannel.sound"],
  [{ channels: [{ id: "c", name: "C", sound: "raw/chime.wav" }] }, "channels[0].sound"],
];

/** Whether `error` is the `TypeError` that names the value at `path`. */
function names(path: string) {
  return (error: unknown) => error instanceof TypeError && error.message.startsWith(`${path} `);
}

void test("rejects each invalid notification value, naming it", () => {
  for (const [value, path] of INVALID_NOTIFICATIONS) {
    assert.throws(() => {
      validateContent(value, false);
    }, names(path), JSON.stringify(value));
  }
  for (const [trigger, path] of INVALID_TRIGGERS) {
    assert.throws(() => {
      validateContent({ ...notification, ...trigger }, true);
    }, names(path), JSON.stringify(trigger));
  }
});

const INVALID_TRIGGERS: [Record<string, unknown>, string][] = [
  [{}, "a scheduled notification"],
  [{ at: 1, every: { hours: 1 } }, "a scheduled notification"],
  [{ at: Number.POSITIVE_INFINITY }, "at"],
  [{ every: { seconds: 59 } }, "every"],
  [{ every: { minutes: 0.5 } }, "every.minutes"],
  [{ every: 60 }, "every"],
  [{ on: {} }, "on"],
  [{ on: { month: 13 } }, "on.month"],
  [{ on: { weekday: 0 } }, "on.weekday"],
  [{ on: { hour: 24 } }, "on.hour"],
  [{ on: { minute: 1.5 } }, "on.minute"],
  [{ region: { latitude: 91, longitude: 0, radius: 100 } }, "region.latitude"],
  [{ region: { latitude: 0, longitude: 0, radius: 0 } }, "region.radius"],
  [{ region: { latitude: 0, longitude: 0 } }, "region.radius"],
  [{ region: { latitude: 0, longitude: 0, radius: 10, onEntry: false } }, "region"],
  [{ at: 1, repeats: true }, "repeats"],
  [{ every: { days: 1 }, repeats: true }, "repeats"],
  [{ region: { latitude: 0, longitude: 0, radius: 10 }, exact: true }, "exact"],
  [{ at: 1, exact: "yes" }, "exact"],
];

void test("accepts each kind of trigger", () => {
  for (const trigger of [
    { at: 0 },
    { at: 1, exact: true },
    { every: { seconds: 60 } },
    { every: { minutes: 1 } },
    { every: { days: 1, hours: 2 }, exact: true },
    { on: { hour: 9, minute: 0 }, repeats: true },
    { on: { year: 2027, month: 2, day: 29, weekday: 7, second: 59 } },
    { region: { latitude: -90, longitude: 180, radius: 0.5 }, repeats: false },
    { region: { latitude: 51.5, longitude: -0.1, radius: 100, onEntry: false, onExit: true } },
  ]) {
    validateContent({ ...notification, ...trigger }, true);
  }
});

void test("accepts boundary notification values", () => {
  for (const value of [
    { progress: { current: 5, max: 5 } },
    { progress: { indeterminate: true } },
    { relevanceScore: 0 },
    { relevanceScore: 1 },
    { interruptionLevel: "critical" },
    { chronometer: false },
    { silent: false, vibrate: [100] },
    { image: "https://example.com/i.png", icon: "/icon.png" },
    { sound: "default" },
    { sound: "defaultCritical" },
    { sound: { name: "Chime Bell.caf" } },
    { sound: { name: "alarm.wav", critical: true, volume: 0 } },
    { settingsText: "Notification settings" },
    { image: "HTTPS://example.com:8443/i.png?size=2", icon: "/icons/a%20b.png?v=1" },
    { attachments: [] },
    { image: "/i.png", style: { type: "bigPicture", title: "Photo", showWhenCollapsed: true } },
    {
      attachments: [
        { url: "/a.png", hideThumbnail: false, thumbnailClippingRect: { x: 0, y: 0, width: 0.5, height: 1 } },
        { url: "https://example.com/a.mp4", thumbnailTime: 2.5 },
        { url: "/b.png", thumbnailClippingRect: { x: 0.7, y: 0.6, width: 0.3, height: 0.4 } },
        { url: "/b.mp4", thumbnailTime: 0 },
      ],
    },
  ]) {
    validateContent({ ...notification, ...value }, false);
  }
});

void test("rejects each invalid setup value, naming it", () => {
  for (const [config, path] of INVALID_SETUPS) {
    assert.throws(() => {
      validateSetup(config);
    }, names(path), JSON.stringify(config));
  }
});

void test("accepts showInForeground as a boolean", () => {
  validateSetup({
    defaultChannel: { name: "General", sound: "none" },
    channels: [{ id: "chimes", name: "Chimes", sound: "chime.wav" }],
  });
  validateSetup({ showInForeground: true });
  validateSetup({ showInForeground: false });
});

void test("rejects invalid options of the other methods, naming them", () => {
  const cases: [() => void, string][] = [
    [() => { validatePermissionOptions({ provisional: "yes" }); }, "options.provisional"],
    [() => { validatePermissionOptions(null); }, "options"],
    [() => { validateBadge(-1); }, "count"],
    [() => { validateBadge(1.5); }, "count"],
    [() => { validateOpenSettings({ channel: "" }); }, "options.channel"],
    [() => { validateOpenSettings({ channel: "c", exactAlarms: true }); }, "options"],
    [() => { validateTopic(""); }, "topic"],
    [() => { validateTopic("news/today"); }, "topic"],
    [() => { validateTopic("x".repeat(901)); }, "topic"],
  ];
  for (const [call, path] of cases) assert.throws(call, names(path), path);
  validatePermissionOptions({ alert: false, carPlay: true, providesAppNotificationSettings: true });
  validateBadge(0);
  validateOpenSettings({ channel: "messages", exactAlarms: false });
  validateTopic("weather-london_2.0~%41");
});
