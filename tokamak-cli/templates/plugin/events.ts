import { defineEvent } from "@tokamakdev/plugin/events";

declare global {
  // eslint-disable-next-line @typescript-eslint/no-namespace -- events are declared by merging into Tokamak.Events.
  namespace Tokamak {
    interface Events {
      "{{id}}.echoed": { readonly message: string };
    }
  }
}

/** Receives each message the native implementation echoed. */
export const onEchoed = defineEvent("{{id}}.echoed");
