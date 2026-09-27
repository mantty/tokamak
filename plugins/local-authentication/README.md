# @tokamakdev/plugin-local-authentication

Device owner authentication for Tokamak applications.

Add it to your project's dependencies:

```sh
npm install @tokamakdev/plugin-local-authentication
```

```ts
import { localAuthentication } from "@tokamakdev/plugin-local-authentication";

// "available" | "notEnrolled" | "unavailable"
const status = await localAuthentication.status("biometricsOrPasscode");

await localAuthentication.authenticate("biometricsOrPasscode", {
  prompt: "Confirm it's you to link a new device",
});
```

Both methods accept `biometricsOrPasscode` (Face ID, Touch ID or fingerprint,
falling back to the device passcode, PIN, pattern or password) or `biometrics`.

`status` reports whether the device can perform the authentication now.
`notEnrolled` means no biometric or passcode is set up.

`authenticate` resolves when the device owner authenticates. It rejects with
`NotAllowedError` when the user cancels or fails, `InvalidStateError` when the
authentication is not set up, and `NotSupportedError` when the device or OS
version cannot perform it.

`authenticate` checks the device owner when the app asks; it does not protect
any data. To bind a stored value to authentication, use
`@tokamakdev/plugin-secure-storage`.

## Platforms

- **iOS and macOS:** LocalAuthentication. On macOS, `biometricsOrPasscode`
  accepts Touch ID or the account password.
- **Android:** the platform `BiometricPrompt`. `biometrics` requires API 28 and
  `biometricsOrPasscode` requires API 29.
- **Web and Windows:** no implementation; calls throw `NotSupportedError`.
