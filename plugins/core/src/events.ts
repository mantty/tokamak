import { decodeJson, encodeJson } from "./codec.js";

declare global {
  // eslint-disable-next-line @typescript-eslint/no-namespace -- plugins add their handlers by declaration merging.
  namespace Tokamak {
    /**
     * The handlers of the Tokamak worker's class, which the native host calls, each with an
     * event's data: the lifecycle handlers, and a property for each plugin, `_<plugin id>`, that
     * holds the handlers of the events the plugin's native code emits. Each handler is an arrow
     * function, whose `this` is the worker.
     */
    interface Handlers {
      /** Runs inside the platform's launch, before `_onStart`, while the app's stage is `background`. */
      _beforeStart?: () => void | Promise<void>;
      /** The runtime started, including when the system started the app in the background. */
      _onStart?: (event: StartEvent) => void | Promise<void>;
      /** The app moved into the foreground, other than when it started there. */
      _onResume?: () => void | Promise<void>;
      /** The app moved out of the foreground. */
      _onSuspend?: () => void | Promise<void>;
    }
  }
}

/** What `_onStart` receives. */
export interface StartEvent {
  readonly foreground: boolean;
}

/** How an event's data and reply travel as bytes. */
export interface EventCodec<T, Reply> {
  decode(payload: Uint8Array): T;
  encode(reply: Reply): Uint8Array;
}

const json: EventCodec<unknown, unknown> = { decode: decodeJson, encode: encodeJson };
const codecs = new Map<string, EventCodec<unknown, unknown>>();

/**
 * Makes the data and reply of the events that call `handler` travel as `codec` encodes them. A
 * plugin's handler is `<plugin id>.<event>`.
 */
export function setEventCodec<T, Reply>(handler: string, codec: EventCodec<T, Reply>): void {
  codecs.set(handler, codec);
}

/** The codec of the events that call `handler`: the one a plugin set, or JSON. */
export function eventCodec(handler: string): EventCodec<unknown, unknown> {
  return codecs.get(handler) ?? json;
}
