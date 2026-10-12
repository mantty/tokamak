import { Plugin } from "@tokamakdev/plugin";

/** When a stored value can be read. */
export type Readable = "whenUnlocked" | "afterFirstUnlock";

/** The authentication a read requires. */
export type Authentication = "biometricsOrPasscode" | "biometrics" | "currentBiometrics";

export interface SetOptions {
  readonly readable: Readable;
  /** Reads need no authentication when omitted. */
  readonly authentication?: Authentication;
  /**
   * Whether backups can restore the value only to this device. Defaults to true;
   * false lets a backup restore it to a new device where the platform supports that.
   */
  readonly thisDeviceOnly?: boolean;
  /**
   * The reason shown in the system authentication prompt. Apple platforms ask for the
   * authentication an existing value was saved with before replacing it.
   */
  readonly prompt?: string;
}

export interface GetOptions {
  /** The reason shown in the system authentication prompt. */
  readonly prompt?: string;
}

class SecureStorage extends Plugin {
  constructor() {
    super("secure-storage");
  }

  set(name: string, value: string, options: SetOptions): Promise<void> {
    return this.call("set", {
      name,
      value,
      readable: options.readable,
      authentication: options.authentication ?? null,
      thisDeviceOnly: options.thisDeviceOnly ?? true,
      prompt: options.prompt ?? null,
    });
  }

  get(name: string, options: GetOptions = {}): Promise<string | null> {
    return this.call("get", { name, prompt: options.prompt ?? null });
  }

  delete(name: string): Promise<void> {
    return this.call("delete", { name });
  }

  /** Deletes every stored value. */
  clear(): Promise<void> {
    return this.call("clear");
  }
}

export const secureStorage = new SecureStorage();
