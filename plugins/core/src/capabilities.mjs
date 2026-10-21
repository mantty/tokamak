/** Whether each operation of each plugin in `availability` has an implementation on `platform`. */
export function capabilitiesFor(availability, platform) {
  return Object.fromEntries(Object.entries(availability).map(([plugin, operations]) => [
    plugin,
    Object.fromEntries(Object.entries(operations).map(([operation, platforms]) =>
      [operation, platform !== undefined && platforms.includes(platform)])),
  ]));
}
