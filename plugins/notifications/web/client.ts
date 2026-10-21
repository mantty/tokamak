import { valueStream, type JsonStream } from "@tokamakdev/plugin/streams";
import * as web from "./index.js";
import type { Message, OpenedNotification, DismissedNotification, Subscription, SetupConfig, SetupResult, NotificationContent, SubscribeOptions } from "../src/types.js";

export const notifications = {
  setup: (config: SetupConfig): Promise<SetupResult> => web.setup(config),
  permission: web.permission,
  requestPermission: web.requestPermission,
  settings: web.settings,
  setBadge: (count: number): Promise<void> => web.setBadge(count),
  show: (notification: NotificationContent): Promise<void> => web.show(notification),
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
