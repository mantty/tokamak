import { Plugin, type TokamakContext } from "@tokamakdev/plugin";

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

class SecureStorage extends Plugin {
  constructor(ctx: TokamakContext) {
    super(ctx, "secure-storage");
  }

  /** Stores a new value; rejects with `DuplicateItemError` when `name` has one. */
  create(name: string, value: string, options: CreateOptions): Promise<void> {
    return this.call("create", stored(name, value, options));
  }

  /**
   * Replaces the value in place; rejects with `ItemNotFoundError` when `name` has none. Replacing a
   * value saved with `authentication` rejects with `NeedsUIError` off screen on iOS.
   */
  replace(name: string, value: string, options: SetOptions): Promise<void> {
    return this.call("replace", { ...stored(name, value, options), prompt: options.prompt ?? null });
  }

  /**
   * Replaces the value in place, or stores a new one when `name` has none. Replacing a value saved
   * with `authentication` rejects with `NeedsUIError` off screen on iOS.
   */
  set(name: string, value: string, options: SetOptions): Promise<void> {
    return this.call("set", { ...stored(name, value, options), prompt: options.prompt ?? null });
  }

  /**
   * Reads the value; rejects with `ItemNotFoundError` when `name` has none. Reading a value saved
   * with `authentication` rejects with `NeedsUIError` off screen on iOS and Android.
   */
  get(name: string, options: GetOptions = {}): Promise<string> {
    return this.call("get", { name, prompt: options.prompt ?? null, keychainGroup: options.keychainGroup ?? null });
  }

  /** Deletes the value; rejects with `ItemNotFoundError` when `name` has none. */
  delete(name: string, options: DeleteOptions = {}): Promise<void> {
    return this.call("delete", { name, keychainGroup: options.keychainGroup ?? null });
  }

  /** Deletes every stored value; rejects with `ItemNotFoundError` when there are none. */
  clear(options: DeleteOptions = {}): Promise<void> {
    return this.call("clear", { keychainGroup: options.keychainGroup ?? null });
  }
}

function stored(name: string, value: string, options: CreateOptions) {
  return {
    name,
    value,
    readable: options.readable,
    authentication: options.authentication ?? null,
    thisDeviceOnly: options.thisDeviceOnly ?? true,
    keychainGroup: options.keychainGroup ?? null,
  };
}

export function secureStorage(ctx: TokamakContext): SecureStorage { return new SecureStorage(ctx); }
