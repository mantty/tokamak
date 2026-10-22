import { Plugin, type TokamakContext } from "@tokamakdev/plugin";

/** The authentication the device owner performs. */
export type Authentication = "biometricsOrPasscode" | "biometrics";

/** Whether the device can perform an authentication now. */
export type Status = "available" | "notEnrolled" | "unavailable";

export interface AuthenticateOptions {
  /** The reason shown in the system authentication prompt. */
  readonly prompt: string;
}

class LocalAuthentication extends Plugin {
  constructor(ctx: TokamakContext) {
    super(ctx, "local-authentication");
  }

  status(authentication: Authentication): Promise<Status> {
    return this.call("status", { authentication });
  }

  /**
   * Asks the device owner to authenticate. Rejects with `NeedsUIError` off screen on iOS and
   * Android, where the system cannot show its prompt.
   */
  authenticate(authentication: Authentication, options: AuthenticateOptions): Promise<void> {
    return this.call("authenticate", { authentication, prompt: options.prompt });
  }
}

export function localAuthentication(ctx: TokamakContext): LocalAuthentication { return new LocalAuthentication(ctx); }
