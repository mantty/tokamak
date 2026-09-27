import { FrontendPlugin } from "@tokamakdev/plugin";

/** When a stored value can be read. */
export type Readable = "whenUnlocked" | "afterFirstUnlock";

/** The authentication a read requires. */
export type Authentication = "biometricsOrPasscode" | "biometrics" | "currentBiometrics";

export interface SetOptions {
  readonly readable: Readable;
  /** Omitted: reads need no authentication. */
  readonly authentication?: Authentication;
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
    });
  }

  get(name: string, options: GetOptions = {}): Promise<string | null> {
    return this.call("get", { name, prompt: options.prompt ?? null });
  }

  delete(name: string): Promise<void> {
    return this.call("delete", { name });
  }
}

export const secureStorage = new SecureStorage();
