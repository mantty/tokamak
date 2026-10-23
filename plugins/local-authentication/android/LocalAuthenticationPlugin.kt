package com.tokamak.plugins.localauthentication

import android.hardware.biometrics.BiometricManager
import android.hardware.biometrics.BiometricManager.Authenticators
import com.tokamak.runtime.TokamakHost
import com.tokamak.runtime.TokamakPlugin
import com.tokamak.runtime.TokamakPluginError
import com.tokamak.runtime.authenticateOwner
import com.tokamak.runtime.requireOwnerAuthentication
import kotlin.coroutines.resume
import kotlinx.coroutines.suspendCancellableCoroutine

/** The authentication the device owner performs. */
enum class Authentication(val authenticators: Int) {
    biometricsOrPasscode(Authenticators.BIOMETRIC_STRONG or Authenticators.DEVICE_CREDENTIAL),
    biometrics(Authenticators.BIOMETRIC_STRONG),
}

/** Whether the device can perform an authentication now. */
enum class Status { available, notEnrolled, unavailable }

class TokamakLocalAuthenticationPlugin(
    private val host: TokamakHost,
) : TokamakPlugin {
    override val id = "local-authentication"

    private val biometrics = host.context.getSystemService(BiometricManager::class.java)

    fun status(authentication: Authentication): Status =
        when (biometrics.canAuthenticate(authentication.authenticators)) {
            BiometricManager.BIOMETRIC_SUCCESS -> Status.available
            BiometricManager.BIOMETRIC_ERROR_NONE_ENROLLED -> Status.notEnrolled
            else -> Status.unavailable
        }

    suspend fun authenticate(authentication: Authentication, prompt: String) {
        if (prompt.isEmpty()) throw TokamakPluginError.typeError("prompt must be a non-empty string")
        host.requireOwnerAuthentication(authentication.authenticators)
        suspendCancellableCoroutine { continuation ->
            val cancel = host.authenticateOwner(prompt, authentication.authenticators, null, { failed ->
                continuation.resumeWith(failed.map {})
            }) { continuation.resume(Unit) }
            continuation.invokeOnCancellation { cancel() }
        }
    }
}
