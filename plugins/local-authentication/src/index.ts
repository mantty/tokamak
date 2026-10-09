import { Plugin } from "@tokamakdev/plugin";

/** The authentication the device owner performs. */
export type Authentication = "biometricsOrPasscode" | "biometrics";

/** Whether the device can perform an authentication now. */
export type Status = "available" | "notEnrolled" | "unavailable";

export interface AuthenticateOptions {
  /** The reason shown in the system authentication prompt. */
  readonly prompt: string;
}

class LocalAuthentication extends Plugin {
  constructor() {
    super("local-authentication");
  }

  status(authentication: Authentication): Promise<Status> {
    return this.call("status", { authentication });
  }

  authenticate(authentication: Authentication, options: AuthenticateOptions): Promise<void> {
    return this.call("authenticate", { authentication, prompt: options.prompt });
  }
}

export const localAuthentication = new LocalAuthentication();
