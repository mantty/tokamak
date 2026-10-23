import type { Listener } from "@tokamakdev/plugin/events";

{{namespaces}}

/** Receives each message the native implementation echoed, from each family's event. */
export function onEchoed(listener: Listener<{{family}}.Echo>): void {
{{listeners}}
}
