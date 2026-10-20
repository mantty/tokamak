import { valueStream, type JsonStream } from "@tokamakdev/plugin/streams";
import * as web from "./index.js";
import { validateContent } from "../src/content.js";
import { validateBadge, validatePermissionOptions } from "../src/options.js";
import { validateSetup } from "../src/setup.js";
import type { Message, OpenedNotification, DismissedNotification, Subscription, SetupConfig, SetupResult, PermissionOptions, Permission, NotificationContent, SubscribeOptions } from "../src/types.js";

export const notifications = {
  async setup(config: SetupConfig): Promise<SetupResult> { validateSetup(config); return web.setup(config); },
  permission: web.permission,
  async requestPermission(options: PermissionOptions = {}): Promise<Permission> { validatePermissionOptions(options); return web.requestPermission(); },
  settings: web.settings,
  async setBadge(count: number): Promise<void> { validateBadge(count); return web.setBadge(count); },
  async show(notification: NotificationContent): Promise<void> { validateContent(notification, false); return web.show(notification); },
  getDelivered: web.getDelivered,
  remove: web.remove,
  subscribe: (options: SubscribeOptions = {}) => web.subscribe(options),
  getSubscription: web.getSubscription,
  unsubscribe: web.unsubscribe,
  onMessage(): Promise<JsonStream<Message>> { return Promise.resolve(valueStream<Message>(next => web.onMessage(next))); },
  onNotificationOpened(): Promise<JsonStream<OpenedNotification>> { return Promise.resolve(valueStream<OpenedNotification>(next => web.onNotificationOpened(next))); },
  onNotificationDismissed(): Promise<JsonStream<DismissedNotification>> { return Promise.resolve(valueStream<DismissedNotification>(next => web.onNotificationDismissed(next))); },
  onSubscriptionChange(): Promise<JsonStream<Subscription>> { return Promise.resolve(valueStream<Subscription>(next => web.onSubscriptionChange(next))); },
};
