// The process object, which emits warnings. It is itself an event emitter, so
// it registers itself here instead of node:events importing it.
let warningProcess: { emit(event: "warning", warning: unknown): unknown } | undefined;

export function setWarningProcess(process: NonNullable<typeof warningProcess>): void {
  warningProcess = process;
}

// Emits `warning` on the process after the current job, as workerd does.
export function emitProcessWarning(warning: unknown): void {
  queueMicrotask(() => warningProcess?.emit("warning", warning));
}
