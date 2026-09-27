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
        val authenticators = AUTHENTICATORS[request?.opt("authentication") as? String]
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
        val callback =
            object : BiometricPrompt.AuthenticationCallback() {
                override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) {
                    reply(Result.success(null))
                }

                override fun onAuthenticationError(code: Int, message: CharSequence) {
                    reply(Result.failure(authenticationError(code, message)))
                }
            }
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

    private companion object {
        val AUTHENTICATORS =
            mapOf(
                "biometricsOrPasscode" to (Authenticators.BIOMETRIC_STRONG or Authenticators.DEVICE_CREDENTIAL),
                "biometrics" to Authenticators.BIOMETRIC_STRONG,
            )

        fun typeError(message: String): Result<Any?> = Result.failure(TokamakPluginError("TypeError", message))

        fun authenticationError(code: Int, message: CharSequence): TokamakPluginError {
            val name =
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
            return TokamakPluginError(name, message.toString())
        }
    }
}
