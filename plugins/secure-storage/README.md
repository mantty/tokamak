# @tokamakdev/plugin-secure-storage

Secret storage that stays on the device, for Tokamak applications.

Add it to your project's dependencies:

```sh
npm install @tokamakdev/plugin-secure-storage
```

```ts
import { secureStorage } from "@tokamakdev/plugin-secure-storage";

await secureStorage.set("identity/encryption", base64Value, {
  // Required: "whenUnlocked" or "afterFirstUnlock".
  readable: "whenUnlocked",
  // Reads need no authentication when omitted.
  authentication: "biometricsOrPasscode",
});

// string | null
const value = await secureStorage.get("identity/encryption", {
  prompt: "Confirm it's you to link a new device",
});

await secureStorage.delete("identity/encryption");

// string[], sorted
const names = await secureStorage.keys();

await secureStorage.clear();
```

Values are strings; encode binary values before storing them. Only the app can
read them, and they are never synced to other devices.

By default a backup restores a value only to the device that stored it. Set
`thisDeviceOnly: false` to let a backup restore it to a new device, where the
platform supports that.

`authentication` binds a value to the device's secure hardware, so it cannot be
decrypted until the device owner authenticates:

| `authentication` | Reads accept | After a biometric enrolment change |
|---|---|---|
| `biometricsOrPasscode` | Face ID, Touch ID or fingerprint; passcode, PIN, pattern or password | Readable |
| `biometrics` | Face ID, Touch ID or fingerprint | Readable |
| `currentBiometrics` | Face ID, Touch ID or fingerprint enrolled when the value was stored | Permanently unreadable |

`prompt` is the reason shown in the system authentication prompt. Without it,
the platform's default prompt is shown.

## Errors

| Name | When |
|---|---|
| `NotSupportedError` | The platform, build or device cannot enforce the options |
| `InvalidStateError` | The requested authentication is not set up on the device |
| `NotAllowedError` | The user cancelled or failed authentication, or the device is locked |
| `NotReadableError` | A stored value can no longer be decrypted |
| `TypeError` | An argument is missing or has an unsupported value |
| `OperationError` | The platform reported another failure |

`get` resolves `null` when the name has no stored value.

## Platforms

- **iOS:** Keychain items, which survive deleting and reinstalling the app.
  They are included in encrypted backups; `thisDeviceOnly` values restore only
  to the same device.
- **macOS:** the data protection keychain, which requires a team-signed build
  (`macos.team-id`). Ad-hoc signed builds throw `NotSupportedError`.
- **Android:** values are encrypted with a per-value Android Keystore key and
  stored in the app's no-backup directory. They are deleted on uninstall.
  Keystore keys never leave the device, so `thisDeviceOnly: false` throws
  `NotSupportedError`.
- **Web and Windows:** no implementation; calls throw `NotSupportedError`.

Removing the device passcode or screen lock makes values stored with
`authentication` permanently unreadable on Android.
