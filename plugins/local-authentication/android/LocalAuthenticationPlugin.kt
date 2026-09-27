package com.tokamak.plugins.localauthentication

import android.app.Activity
import android.hardware.biometrics.BiometricManager
import android.hardware.biometrics.BiometricManager.Authenticators
import android.hardware.biometrics.BiometricPrompt
import android.os.CancellationSignal
import com.tokamak.runtime.TokamakPlugin
import com.tokamak.runtime.TokamakPluginError
import com.tokamak.runtime.TokamakPluginReply
import org.json.JSONObject

internal class TokamakLocalAuthenticationPlugin(
    private val activity: Activity,
) : TokamakPlugin {
    override val id = "local-authentication"

    private val biometrics = activity.getSystemService(BiometricManager::class.java)

    override fun call(method: String, arguments: Any?, reply: TokamakPluginReply) {
        if (method != "status" && method != "authenticate") {
            super.call(method, arguments, reply)
            return
        }
        val request = arguments as? JSONObject
        val wireName = request?.opt("authentication") as? String
        val authenticators = Authentication.entries.firstOrNull { it.wireName == wireName }?.authenticators
        val prompt = request?.opt("prompt") as? String
        when {
            authenticators == null ->
                reply(typeError("authentication must be \"biometricsOrPasscode\" or \"biometrics\""))
            method == "status" -> reply(Result.success(status(authenticators)))
            prompt.isNullOrEmpty() -> reply(typeError("prompt must be a non-empty string"))
            else -> authenticate(authenticators, prompt, reply)
        }
    }

    private fun status(authenticators: Int): String =
        when (biometrics.canAuthenticate(authenticators)) {
            BiometricManager.BIOMETRIC_SUCCESS -> "available"
            BiometricManager.BIOMETRIC_ERROR_NONE_ENROLLED -> "notEnrolled"
            else -> "unavailable"
        }

    private fun authenticate(authenticators: Int, prompt: String, reply: TokamakPluginReply) {
        val callback = PromptCallback(reply) { reply(Result.success(null)) }
        val dialog =
            BiometricPrompt.Builder(activity)
                .setTitle(prompt)
                .setAllowedAuthenticators(authenticators)
        if ((authenticators and Authenticators.DEVICE_CREDENTIAL) == 0) {
            dialog.setNegativeButton(
                activity.getString(android.R.string.cancel),
                activity.mainExecutor,
            ) { _, _ ->
                reply(Result.failure(TokamakPluginError("NotAllowedError", "Authentication was cancelled")))
            }
        }
        dialog.build().authenticate(CancellationSignal(), activity.mainExecutor, callback)
    }

    private enum class Authentication(val wireName: String, val authenticators: Int) {
        BIOMETRICS_OR_PASSCODE(
            "biometricsOrPasscode",
            Authenticators.BIOMETRIC_STRONG or Authenticators.DEVICE_CREDENTIAL,
        ),
        BIOMETRICS("biometrics", Authenticators.BIOMETRIC_STRONG),
    }

    private companion object {
        fun typeError(message: String): Result<Any?> = Result.failure(TokamakPluginError("TypeError", message))
    }
}

private class PromptCallback(
    private val reply: TokamakPluginReply,
    private val succeeded: () -> Unit,
) : BiometricPrompt.AuthenticationCallback() {
    override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) = succeeded()

    override fun onAuthenticationError(code: Int, message: CharSequence) =
        reply(Result.failure(authenticationError(code, message)))
}

private fun authenticationError(code: Int, message: CharSequence) =
    TokamakPluginError(
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
        },
        message.toString(),
    )
