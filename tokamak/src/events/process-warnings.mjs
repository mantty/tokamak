// The process object, which emits warnings. It is itself an event emitter, so
// it registers itself here instead of node:events importing it.
let warningProcess;

export function setWarningProcess(process) {
  warningProcess = process;
}

// Emits `warning` on the process after the current job, as workerd does.
export function emitProcessWarning(warning) {
  queueMicrotask(() => warningProcess?.emit("warning", warning));
}
