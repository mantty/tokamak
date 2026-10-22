# @tokamakdev/plugin-secure-storage

Secret storage that stays on the device, for Tokamak applications.

Add it to your project's dependencies:

```sh
npm install @tokamakdev/plugin-secure-storage
```

Pages import the generated `/client` SDK. In `src/tokamak.ts`, import the
package root and call its factory with the handler's `ctx`. The main worker
uses its generated `env.TOKAMAK_SECURE_STORAGE` service binding when present.

```ts
import secureStorage from "@tokamakdev/plugin-secure-storage/client";

const options = {
  // Required: "whenUnlocked" or "afterFirstUnlock".
  readable: "whenUnlocked",
  // Reads need no authentication when omitted.
  authentication: "biometricsOrPasscode",
} as const;

// Rejects with DuplicateItemError when the name has a value.
await secureStorage.create("identity/encryption", base64Value, options);

// Rejects with ItemNotFoundError when the name has no value.
await secureStorage.replace("identity/encryption", base64Value, {
  ...options,
  prompt: "Confirm it's you to replace your key",
});

// Replaces the value, or creates it when the name has none.
await secureStorage.set("identity/encryption", base64Value, options);

// get, delete and clear reject with ItemNotFoundError when there is no value.
const value = await secureStorage.get("identity/encryption", {
  prompt: "Confirm it's you to link a new device",
});

await secureStorage.delete("identity/encryption");

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

On iOS, the system reports a read of a `currentBiometrics` value after an
enrolment change as failed authentication, so `get` rejects with
`NotAllowedError` rather than `NotReadableError`.

`prompt` is the reason shown in the system authentication prompt. Without it,
the platform's default prompt is shown.

`replace` and `set` replace an existing value in place, so a failed write
leaves the previous value. On Apple platforms the system asks for the
authentication an existing value was saved with before replacing it, as it does
for a read: `replace` and `set` then show their `prompt` and need the app on
screen. Android replaces values without asking.

## Errors

| Name | When |
|---|---|
| `NotSupportedError` | The platform, build or device cannot enforce the options |
| `DuplicateItemError` | `create` was given a name that has a value |
| `ItemNotFoundError` | `replace`, `get` or `delete` was given a name that has no value, or `clear` found no values |
| `MissingEntitlementError` | `keychainGroup` is not one of the app's keychain access groups |
| `InvalidStateError` | The requested authentication is not set up on the device |
| `NotAllowedError` | The user cancelled or failed authentication, or the device is locked |
| `NeedsUIError` | Reading a value saved with `authentication` while the app is off screen on iOS or Android, or replacing one on iOS, where the system cannot show its prompt |
| `NotReadableError` | A stored value can no longer be decrypted |
| `TypeError` | An argument is missing or has an unsupported value |
| `OperationError` | The platform reported another failure |

## Keychain access groups

On Apple platforms an app can store values in each keychain access group its
`keychain-access-groups` entitlement lists. `keychainGroup` selects one, as
`kSecAttrAccessGroup` does in a native keychain query:

- `create`, and `set` when the name has no value, store the value in
  `keychainGroup`, or in the first listed group when it is omitted.
- `get`, `replace`, `delete` and `clear`, and `set` when the name has a value,
  act on `keychainGroup`, or on every group when it is omitted.

A name can have a value in more than one group. Without `keychainGroup`, `get`
resolves one of them, `delete` and `clear` delete all of them, and `replace`
and `set` reject with `NotAllowedError`, which is how the keychain reports an
update that matches more than one value (`errSecAuthFailed`).

Other platforms have no equivalent and reject `keychainGroup` with
`NotSupportedError`.

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

### Values stored by builds from tok 0.1.1-beta.87 or earlier

Team-signed iOS and macOS builds made with tok 0.1.1-beta.87 or earlier kept
values in a keychain access group literally named `TEAMID.*`, which all of a
team's apps built that way share. Later builds use the app's own group,
`TEAMID.<bundle id>`, and cannot see those values. To keep them, declare both
groups, the app's own first, in the app's entitlements file
(`ios.entitlements`, `macos.entitlements`):

```xml
<key>keychain-access-groups</key>
<array>
  <string>TEAMID.com.example.app</string>
  <string>TEAMID.*</string>
</array>
```

New values go into the app's own group; existing values stay in the shared
group even when replaced, so keep `TEAMID.*` while users may still hold them.
Pass `keychainGroup: "TEAMID.*"` to act on those values only, for example to
delete one after creating it again in the app's own group.

Removing the device passcode or screen lock makes values stored with
`authentication` permanently unreadable on Android.
