/** Appends each line, with the time, to the element with `id`. */
export function logTo(id: string): (line: string) => void {
  const element = document.getElementById(id)!;
  return (line) => {
    const entry = document.createElement("div");
    entry.textContent = `[${new Date().toISOString().slice(11, 19)}] ${line}`;
    element.append(entry);
    element.scrollTop = element.scrollHeight;
  };
}

/** A plugin error, which is a DOMException, as its name and message. */
export function describe(error: unknown): string {
  return error instanceof DOMException ? `${error.name}: ${error.message}` : String(error);
}

/** Runs `operation` on each click of the button `id`, logging how it ends after the button's text. */
export function onClick(log: (line: string) => void, id: string, operation: () => Promise<unknown>): void {
  const button = document.getElementById(id)!;
  const label = button.textContent!.trim();
  button.addEventListener("click", async () => {
    try {
      const value = await operation();
      log(`${label}: ${typeof value === "string" ? value : (JSON.stringify(value) ?? "done")}`);
    } catch (error) {
      log(`${label}: ${describe(error)}`);
    }
  });
}
