import { capabilitiesOf, platformApi, type TokamakContext } from "@tokamakdev/plugin";

import { android } from "./android.generated.js";
import { apple } from "./apple.generated.js";

/** When a stored value can be read. */
export type Readable = "whenUnlocked" | "afterFirstUnlock";

/** The authentication a read requires. */
export type Authentication = "biometricsOrPasscode" | "biometrics" | "currentBiometrics";

export interface CreateOptions {
  readonly readable: Readable;
  /** Reads need no authentication when omitted. */
  readonly authentication?: Authentication;
  /**
   * Whether backups can restore the value only to this device. Defaults to true;
   * false lets a backup restore it to a new device where the platform supports that.
   */
  readonly thisDeviceOnly?: boolean;
  /**
   * Apple platforms: the keychain access group, one of the app's `keychain-access-groups`.
   * When omitted, new values go into the first of them and updates match every group.
   */
  readonly keychainGroup?: string;
}

/** Options for `set` and `replace`. */
export interface SetOptions extends CreateOptions {
  /**
   * The reason shown in the system authentication prompt. Apple platforms ask for the
   * authentication an existing value was saved with before replacing it.
   */
  readonly prompt?: string;
}

export interface GetOptions {
  /** The reason shown in the system authentication prompt. */
  readonly prompt?: string;
  /** Apple platforms: the keychain access group to read from. Defaults to all of the app's. */
  readonly keychainGroup?: string;
}

/** Options for `delete` and `clear`. */
export interface DeleteOptions {
  /** Apple platforms: the keychain access group to delete from. Defaults to all of the app's. */
  readonly keychainGroup?: string;
}

/** `options` with `thisDeviceOnly`, which is true unless they say otherwise. */
function thisDeviceOnly<Options extends CreateOptions>(options: Options) {
  return { ...options, thisDeviceOnly: options.thisDeviceOnly ?? true };
}

export function secureStorage(ctx: TokamakContext) {
  const native = platformApi(ctx, { apple, android });
  return {
    /** Whether each operation is implemented where the app runs, without calling it. Permission is not reported. */
    get capabilities(): Readonly<Record<string, boolean>> {
      return capabilitiesOf(ctx, "secure-storage");
    },

    /** Stores a new value; rejects with `DuplicateItemError` when `name` has one. */
    create: (name: string, value: string, options: CreateOptions): Promise<void> =>
      native.create(name, value, thisDeviceOnly(options)),

    /**
     * Replaces the value in place; rejects with `ItemNotFoundError` when `name` has none. Replacing a
     * value saved with `authentication` rejects with `NeedsUIError` off screen on iOS.
     */
    replace: (name: string, value: string, options: SetOptions): Promise<void> =>
      native.replace(name, value, thisDeviceOnly(options)),

    /**
     * Replaces the value in place, or stores a new one when `name` has none. Replacing a value saved
     * with `authentication` rejects with `NeedsUIError` off screen on iOS.
     */
    set: (name: string, value: string, options: SetOptions): Promise<void> =>
      native.set(name, value, thisDeviceOnly(options)),

    /**
     * Reads the value; rejects with `ItemNotFoundError` when `name` has none. Reading a value saved
     * with `authentication` rejects with `NeedsUIError` off screen on iOS and Android.
     */
    get: (name: string, options?: GetOptions): Promise<string> => native.get(name, options),

    /** Deletes the value; rejects with `ItemNotFoundError` when `name` has none. */
    delete: (name: string, options?: DeleteOptions): Promise<void> => native.delete(name, options),

    /** Deletes every stored value; rejects with `ItemNotFoundError` when there are none. */
    clear: (options?: DeleteOptions): Promise<void> => native.clear(options),
  };
}
