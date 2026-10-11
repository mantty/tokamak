import assert from "node:assert/strict";
import { test } from "node:test";

import { validateContent } from "../src/content.js";
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
  [{ ...notification, style: { type: "bigPicture" } }, "style.type"],
  [{ ...notification, style: { type: "toString" } }, "style.type"],
  [{ ...notification, style: { type: "bigText" } }, "style.text"],
  [{ ...notification, style: { type: "bigText", text: "t", title: 1 } }, "style.title"],
  [{ ...notification, style: { type: "inbox" } }, "style.lines"],
  [{ ...notification, style: { type: "inbox", lines: [1] } }, "style.lines"],
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
  for (const at of [undefined, Number.POSITIVE_INFINITY]) {
    assert.throws(() => {
      validateContent({ ...notification, at }, true);
    }, names("at"));
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
  validateSetup({ showInForeground: true });
  validateSetup({ showInForeground: false });
});
