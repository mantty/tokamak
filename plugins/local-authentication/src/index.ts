import { capabilitiesOf, platformApi, type TokamakContext } from "@tokamakdev/plugin";

import { android } from "./android.generated.js";
import { apple } from "./apple.generated.js";

/** The authentication the device owner performs. */
export type Authentication = "biometricsOrPasscode" | "biometrics";

/** Whether the device can perform an authentication now. */
export type Status = "available" | "notEnrolled" | "unavailable";

export interface AuthenticateOptions {
  /** The reason shown in the system authentication prompt. */
  readonly prompt: string;
}

export function localAuthentication(ctx: TokamakContext) {
  const native = platformApi(ctx, { apple, android });
  return {
    /** Whether each operation is implemented where the app runs, without calling it. Permission is not reported. */
    get capabilities(): Readonly<Record<string, boolean>> {
      return capabilitiesOf(ctx, "local-authentication");
    },

    status: (authentication: Authentication): Promise<Status> => native.status(authentication),

    /**
     * Asks the device owner to authenticate. Rejects with `NeedsUIError` off screen on iOS and
     * Android, where the system cannot show its prompt.
     */
    authenticate: (authentication: Authentication, options: AuthenticateOptions): Promise<void> =>
      native.authenticate(authentication, options.prompt),
  };
}
