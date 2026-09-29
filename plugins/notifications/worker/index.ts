import { acceptsRuntimeCalls } from "@tokamakdev/plugin/worker";

import type { Message, NotificationContent } from "../src/index.js";

/** Runs for a data-only push message and returns a notification to show, or nothing. */
export type PushHandler<Data extends object = Record<string, unknown>> =
  | ((message: Message<Data>) => NotificationContent | undefined | Promise<NotificationContent | undefined>)
  | ((message: Message<Data>) => void | Promise<void>);

/**
 * Serves `POST /tokamak/push`: runs `handler` with the data-only push message the tokamak
 * runtime delivers and responds with the notification it returns. Responds 404 outside a
 * tokamak app and development.
 */
export async function handlePushRequest<Data extends object = Record<string, unknown>>(
  request: Request,
  handler: PushHandler<Data>,
): Promise<Response> {
  if (!acceptsRuntimeCalls()) return new Response(null, { status: 404 });
  const message = (await request.json()) as Message<Data>;
  return Response.json((await handler(message)) ?? null);
}
