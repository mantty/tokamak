package com.tokamak.plugins.localauthentication

import android.hardware.biometrics.BiometricManager
import android.hardware.biometrics.BiometricManager.Authenticators
import com.tokamak.runtime.TokamakHost
import com.tokamak.runtime.TokamakPlugin
import com.tokamak.runtime.TokamakPluginError
import com.tokamak.runtime.TokamakPluginReply
import com.tokamak.runtime.authenticateOwner
import com.tokamak.runtime.optionalString
import com.tokamak.runtime.requireObject
import com.tokamak.runtime.requireOwnerAuthentication
import com.tokamak.runtime.requireString
import org.json.JSONObject

class TokamakLocalAuthenticationPlugin(
    private val host: TokamakHost,
) : TokamakPlugin {
    override val id = "local-authentication"

    private val biometrics = host.context.getSystemService(BiometricManager::class.java)

    override fun call(method: String, arguments: Any?, reply: TokamakPluginReply) {
        when (method) {
            "status" -> reply(Result.success(status(requireObject(arguments))))
            "authenticate" -> authenticate(requireObject(arguments), reply)
            else -> super.call(method, arguments, reply)
        }
    }

    private fun status(request: JSONObject): String =
        when (biometrics.canAuthenticate(authenticators(request))) {
            BiometricManager.BIOMETRIC_SUCCESS -> "available"
            BiometricManager.BIOMETRIC_ERROR_NONE_ENROLLED -> "notEnrolled"
            else -> "unavailable"
        }

    private fun authenticate(request: JSONObject, reply: TokamakPluginReply) {
        val authenticators = authenticators(request)
        val prompt = request.requireString("prompt")
        if (prompt.isEmpty()) throw TokamakPluginError.typeError("prompt must be a non-empty string")
        host.requireOwnerAuthentication(authenticators)
        host.authenticateOwner(prompt, authenticators, null, reply) { reply(Result.success(null)) }
    }

    private fun authenticators(request: JSONObject): Int {
        val wireName = request.optionalString("authentication")
        return Authentication.entries.firstOrNull { it.wireName == wireName }?.authenticators
            ?: throw TokamakPluginError.typeError("authentication must be \"biometricsOrPasscode\" or \"biometrics\"")
    }

    private enum class Authentication(val wireName: String, val authenticators: Int) {
        BIOMETRICS_OR_PASSCODE(
            "biometricsOrPasscode",
            Authenticators.BIOMETRIC_STRONG or Authenticators.DEVICE_CREDENTIAL,
        ),
        BIOMETRICS("biometrics", Authenticators.BIOMETRIC_STRONG),
    }
}
