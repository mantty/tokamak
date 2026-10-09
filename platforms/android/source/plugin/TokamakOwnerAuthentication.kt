package com.tokamak.runtime

import android.hardware.biometrics.BiometricManager
import android.hardware.biometrics.BiometricManager.Authenticators
import android.hardware.biometrics.BiometricPrompt
import android.os.CancellationSignal

/**
 * Throws unless the device owner can authenticate with [authenticators]: `NotSupportedError`
 * without the hardware, otherwise `InvalidStateError`.
 */
fun TokamakHost.requireOwnerAuthentication(authenticators: Int) {
    when (context.getSystemService(BiometricManager::class.java).canAuthenticate(authenticators)) {
        BiometricManager.BIOMETRIC_SUCCESS -> Unit
        BiometricManager.BIOMETRIC_ERROR_NO_HARDWARE ->
            throw TokamakPluginError.notSupported("This device cannot perform the requested authentication")
        else -> throw TokamakPluginError("InvalidStateError", "The requested authentication is not set up on this device")
    }
}

/**
 * Shows the system prompt, titled [title], for the device owner to authenticate with
 * [authenticators], authorising [crypto] when given. Calls [succeeded] once they do; otherwise
 * [reply] receives the failure. Throws `NeedsUIError` while the app is in the background. Call it
 * on the main thread.
 */
fun TokamakHost.authenticateOwner(
    title: CharSequence,
    authenticators: Int,
    crypto: BiometricPrompt.CryptoObject?,
    reply: TokamakPluginReply,
    succeeded: () -> Unit,
) {
    val activity = requireForegroundActivity()
    val builder = BiometricPrompt.Builder(activity).setTitle(title).setAllowedAuthenticators(authenticators)
    if ((authenticators and Authenticators.DEVICE_CREDENTIAL) == 0) {
        builder.setNegativeButton(activity.getString(android.R.string.cancel), activity.mainExecutor) { _, _ ->
            reply(Result.failure(TokamakPluginError("NotAllowedError", "Authentication was cancelled")))
        }
    }
    val prompt = builder.build()
    val callback = PromptCallback(reply, succeeded)
    if (crypto == null) {
        prompt.authenticate(CancellationSignal(), activity.mainExecutor, callback)
    } else {
        prompt.authenticate(crypto, CancellationSignal(), activity.mainExecutor, callback)
    }
}

private class PromptCallback(
    private val reply: TokamakPluginReply,
    private val succeeded: () -> Unit,
) : BiometricPrompt.AuthenticationCallback() {
    override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) = succeeded()

    override fun onAuthenticationError(code: Int, message: CharSequence) =
        reply(Result.failure(TokamakPluginError(authenticationErrorName(code), message.toString())))
}

private fun authenticationErrorName(code: Int): String =
    when (code) {
        BiometricPrompt.BIOMETRIC_ERROR_CANCELED,
        BiometricPrompt.BIOMETRIC_ERROR_USER_CANCELED,
        BiometricPrompt.BIOMETRIC_ERROR_TIMEOUT,
        BiometricPrompt.BIOMETRIC_ERROR_LOCKOUT,
        BiometricPrompt.BIOMETRIC_ERROR_LOCKOUT_PERMANENT,
        -> "NotAllowedError"
        BiometricPrompt.BIOMETRIC_ERROR_NO_BIOMETRICS,
        BiometricPrompt.BIOMETRIC_ERROR_NO_DEVICE_CREDENTIAL,
        -> "InvalidStateError"
        BiometricPrompt.BIOMETRIC_ERROR_HW_NOT_PRESENT -> "NotSupportedError"
        else -> "OperationError"
    }
