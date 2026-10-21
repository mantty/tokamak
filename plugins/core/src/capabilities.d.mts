/** A native platform the host runs on. */
export type NativePlatform = "ios" | "macos" | "android" | "windows" | "linux";

/** The native platforms of each plugin operation, by plugin ID and operation name. */
export type Availability = Readonly<Record<string, Readonly<Record<string, readonly NativePlatform[]>>>>;

/** Whether each operation of each plugin in `availability` has an implementation on `platform`. */
export function capabilitiesFor(
  availability: Availability,
  platform: NativePlatform | undefined,
): Record<string, Record<string, boolean>>;
