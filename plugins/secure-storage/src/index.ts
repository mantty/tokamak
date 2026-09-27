import { FrontendPlugin } from "@tokamakdev/plugin";

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
}

export interface GetOptions {
  /** The reason shown in the system authentication prompt. */
  readonly prompt?: string;
}

class SecureStorage extends FrontendPlugin {
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
    });
  }

  get(name: string, options: GetOptions = {}): Promise<string | null> {
    return this.call("get", { name, prompt: options.prompt ?? null });
  }

  delete(name: string): Promise<void> {
    return this.call("delete", { name });
  }

  /** The names of all stored values, sorted. */
  keys(): Promise<string[]> {
    return this.call("keys");
  }

  /** Deletes every stored value. */
  clear(): Promise<void> {
    return this.call("clear");
  }
}

export const secureStorage = new SecureStorage();
