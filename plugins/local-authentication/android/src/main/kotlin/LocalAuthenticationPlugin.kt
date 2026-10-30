package com.tokamak.plugins.localauthentication

import android.hardware.biometrics.BiometricManager
import android.hardware.biometrics.BiometricManager.Authenticators
import com.tokamak.runtime.TokamakHost
import com.tokamak.runtime.TokamakPlugin
import com.tokamak.runtime.TokamakPluginError
import com.tokamak.runtime.authenticateOwner
import com.tokamak.runtime.requireOwnerAuthentication

/** The authentication the device owner performs. */
enum class Authentication(val authenticators: Int) {
    BiometricsOrPasscode(Authenticators.BIOMETRIC_STRONG or Authenticators.DEVICE_CREDENTIAL),
    Biometrics(Authenticators.BIOMETRIC_STRONG),
}

/** Whether the device can perform an authentication now. */
enum class Status { Available, NotEnrolled, Unavailable }

class TokamakLocalAuthenticationPlugin(
    private val host: TokamakHost,
) : TokamakPlugin {
    override val id = "local-authentication"

    private val biometrics = host.context.getSystemService(BiometricManager::class.java)

    fun status(authentication: Authentication): Status =
        when (biometrics.canAuthenticate(authentication.authenticators)) {
            BiometricManager.BIOMETRIC_SUCCESS -> Status.Available
            BiometricManager.BIOMETRIC_ERROR_NONE_ENROLLED -> Status.NotEnrolled
            else -> Status.Unavailable
        }

    suspend fun authenticate(authentication: Authentication, prompt: String) {
        if (prompt.isEmpty()) throw TokamakPluginError.typeError("prompt must be a non-empty string")
        host.requireOwnerAuthentication(authentication.authenticators)
        host.authenticateOwner(prompt, authentication.authenticators)
    }
}
