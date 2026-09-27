package com.tokamak.plugins.securestorage

import android.app.Activity
import android.hardware.biometrics.BiometricManager
import android.hardware.biometrics.BiometricManager.Authenticators
import android.hardware.biometrics.BiometricPrompt
import android.os.CancellationSignal
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyInfo
import android.security.keystore.KeyPermanentlyInvalidatedException
import android.security.keystore.KeyProperties
import android.security.keystore.StrongBoxUnavailableException
import android.security.keystore.UserNotAuthenticatedException
import android.util.AtomicFile
import com.tokamak.runtime.TokamakPlugin
import com.tokamak.runtime.TokamakPluginError
import com.tokamak.runtime.TokamakPluginReply
import java.io.File
import java.io.FileNotFoundException
import java.security.KeyFactory
import java.security.KeyPairGenerator
import java.security.KeyStore
import java.security.MessageDigest
import java.security.PrivateKey
import java.security.PublicKey
import java.security.SecureRandom
import java.security.spec.MGF1ParameterSpec
import java.security.spec.X509EncodedKeySpec
import java.util.concurrent.Executors
import javax.crypto.BadPaddingException
import javax.crypto.Cipher
import javax.crypto.IllegalBlockSizeException
import javax.crypto.KeyGenerator
import javax.crypto.spec.GCMParameterSpec
import javax.crypto.spec.OAEPParameterSpec
import javax.crypto.spec.PSource
import javax.crypto.spec.SecretKeySpec
import org.json.JSONObject

/**
 * Stores each value sealed with a random AES-GCM key, wrapped by an RSA key pair in the
 * Android Keystore. Writing uses only the public key, so it never prompts; the private key
 * carries the value's unlock and authentication requirements.
 */
internal class TokamakSecureStoragePlugin(
    private val activity: Activity,
) : TokamakPlugin {
    override val id = "secure-storage"

    private val worker = Executors.newSingleThreadExecutor()
    private val directory = File(activity.noBackupFilesDir, "tokamak-secure-storage")
    private val keyStore = KeyStore.getInstance(KEYSTORE).apply { load(null) }
    private val biometrics = activity.getSystemService(BiometricManager::class.java)

    override fun call(method: String, arguments: Any?, reply: TokamakPluginReply) {
        when (method) {
            "set" -> execute(reply) {
                set(request(arguments))
                reply(Result.success(null))
            }
            "get" -> execute(reply) { get(request(arguments), reply) }
            "delete" -> execute(reply) {
                delete(request(arguments).requireString("name"))
                reply(Result.success(null))
            }
            else -> super.call(method, arguments, reply)
        }
    }

    private fun execute(reply: TokamakPluginReply, operation: () -> Unit) =
        worker.execute {
            runCatching(operation).onFailure { reply(Result.failure(pluginError(it))) }
        }

    private fun set(request: JSONObject) {
        val name = request.requireString("name")
        val value = request.requireString("value")
        val unlockedDeviceRequired =
            when (request.requireString("readable")) {
                "whenUnlocked" -> true
                "afterFirstUnlock" -> false
                else -> throw typeError("readable must be \"whenUnlocked\" or \"afterFirstUnlock\"")
            }
        val spec =
            KeyGenParameterSpec.Builder(alias(name), KeyProperties.PURPOSE_DECRYPT)
                .setKeySize(KEY_SIZE)
                .setDigests(KeyProperties.DIGEST_SHA256)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_RSA_OAEP)
                .setUnlockedDeviceRequired(unlockedDeviceRequired)
        request.optionalString("authentication")?.let { requireAuthentication(spec, it) }
        write(file(name), seal(value, generatePublicKey(spec)))
    }

    private fun requireAuthentication(spec: KeyGenParameterSpec.Builder, wireName: String) {
        val authentication =
            Authentication.entries.firstOrNull { it.wireName == wireName }
                ?: throw typeError(
                    "authentication must be \"biometricsOrPasscode\", \"biometrics\" or \"currentBiometrics\"",
                )
        when (biometrics.canAuthenticate(authentication.authenticators)) {
            BiometricManager.BIOMETRIC_SUCCESS -> Unit
            BiometricManager.BIOMETRIC_ERROR_NONE_ENROLLED ->
                throw TokamakPluginError(
                    "InvalidStateError",
                    "The requested authentication is not set up on this device",
                )
            else -> throw TokamakPluginError.notSupported("This device cannot perform the requested authentication")
        }
        spec.setUserAuthenticationRequired(true)
            .setUserAuthenticationParameters(0, authentication.keyTypes)
            .setInvalidatedByBiometricEnrollment(authentication.invalidatedByEnrollment)
    }

    private fun generatePublicKey(spec: KeyGenParameterSpec.Builder): PublicKey {
        val generator = KeyPairGenerator.getInstance(KeyProperties.KEY_ALGORITHM_RSA, KEYSTORE)
        val keyPair =
            try {
                generator.initialize(spec.setIsStrongBoxBacked(true).build())
                generator.generateKeyPair()
            } catch (_: StrongBoxUnavailableException) {
                generator.initialize(spec.setIsStrongBoxBacked(false).build())
                generator.generateKeyPair()
            }
        // A software copy of the public key encrypts without Keystore restrictions.
        return KeyFactory.getInstance(KeyProperties.KEY_ALGORITHM_RSA)
            .generatePublic(X509EncodedKeySpec(keyPair.public.encoded))
    }

    private fun seal(value: String, publicKey: PublicKey): ByteArray {
        val dataKey = KeyGenerator.getInstance("AES").apply { init(DATA_KEY_SIZE) }.generateKey()
        val iv = ByteArray(IV_SIZE).also(SecureRandom()::nextBytes)
        val sealed =
            Cipher.getInstance(SEAL)
                .apply { init(Cipher.ENCRYPT_MODE, dataKey, GCMParameterSpec(TAG_SIZE, iv)) }
                .doFinal(value.toByteArray())
        val wrapped =
            Cipher.getInstance(WRAP)
                .apply { init(Cipher.ENCRYPT_MODE, publicKey, OAEP) }
                .doFinal(dataKey.encoded)
        return wrapped + iv + sealed
    }

    private fun get(request: JSONObject, reply: TokamakPluginReply) {
        val name = request.requireString("name")
        val stored =
            try {
                AtomicFile(file(name)).readFully()
            } catch (_: FileNotFoundException) {
                reply(Result.success(null))
                return
            }
        val privateKey =
            keyStore.getKey(alias(name), null) as? PrivateKey
                ?: throw notReadable()
        val unwrap = Cipher.getInstance(WRAP).apply { init(Cipher.DECRYPT_MODE, privateKey, OAEP) }
        val info =
            KeyFactory.getInstance(privateKey.algorithm, KEYSTORE)
                .getKeySpec(privateKey, KeyInfo::class.java)
        if (!info.isUserAuthenticationRequired) {
            reply(Result.success(open(stored, unwrap)))
            return
        }
        val credential = (info.userAuthenticationType and KeyProperties.AUTH_DEVICE_CREDENTIAL) != 0
        val authenticators =
            if (credential) Authenticators.BIOMETRIC_STRONG or Authenticators.DEVICE_CREDENTIAL
            else Authenticators.BIOMETRIC_STRONG
        authenticate(authenticators, request.optionalString("prompt"), unwrap, stored, reply)
    }

    /** Authorises [unwrap] with the device owner's authentication, then replies with the opened value. */
    private fun authenticate(
        authenticators: Int,
        prompt: String?,
        unwrap: Cipher,
        stored: ByteArray,
        reply: TokamakPluginReply,
    ) {
        val callback = PromptCallback(reply) { execute(reply) { reply(Result.success(open(stored, unwrap))) } }
        val dialog =
            BiometricPrompt.Builder(activity)
                .setTitle(prompt ?: activity.applicationInfo.loadLabel(activity.packageManager))
                .setAllowedAuthenticators(authenticators)
        if ((authenticators and Authenticators.DEVICE_CREDENTIAL) == 0) {
            dialog.setNegativeButton(
                activity.getString(android.R.string.cancel),
                activity.mainExecutor,
            ) { _, _ -> reply(Result.failure(cancelled())) }
        }
        activity.runOnUiThread {
            dialog.build().authenticate(
                BiometricPrompt.CryptoObject(unwrap),
                CancellationSignal(),
                activity.mainExecutor,
                callback,
            )
        }
    }

    private fun open(stored: ByteArray, unwrap: Cipher): String {
        val sealedStart = WRAPPED_KEY_SIZE + IV_SIZE
        val dataKey = SecretKeySpec(unwrap.doFinal(stored, 0, WRAPPED_KEY_SIZE), "AES")
        val iv = stored.copyOfRange(WRAPPED_KEY_SIZE, sealedStart)
        return Cipher.getInstance(SEAL)
            .apply { init(Cipher.DECRYPT_MODE, dataKey, GCMParameterSpec(TAG_SIZE, iv)) }
            .doFinal(stored, sealedStart, stored.size - sealedStart)
            .decodeToString()
    }

    private fun delete(name: String) {
        AtomicFile(file(name)).delete()
        keyStore.deleteEntry(alias(name))
    }

    private fun write(file: File, bytes: ByteArray) {
        directory.mkdirs()
        val atomic = AtomicFile(file)
        val output = atomic.startWrite()
        try {
            output.write(bytes)
            atomic.finishWrite(output)
        } catch (error: Throwable) {
            atomic.failWrite(output)
            throw error
        }
    }

    private fun file(name: String) = File(directory, digest(name))

    private fun alias(name: String) = "tokamak-secure-storage-${digest(name)}"

    private fun digest(name: String): String =
        MessageDigest.getInstance("SHA-256")
            .digest(name.toByteArray())
            .joinToString("") { "%02x".format(it) }

    private fun request(arguments: Any?): JSONObject =
        arguments as? JSONObject ?: throw typeError("$id arguments must be an object")

    private fun JSONObject.requireString(name: String): String =
        optionalString(name) ?: throw typeError("$name must be a string")

    private fun JSONObject.optionalString(name: String): String? = opt(name) as? String

    private enum class Authentication(
        val wireName: String,
        val keyTypes: Int,
        val authenticators: Int,
        val invalidatedByEnrollment: Boolean,
    ) {
        BIOMETRICS_OR_PASSCODE(
            "biometricsOrPasscode",
            KeyProperties.AUTH_BIOMETRIC_STRONG or KeyProperties.AUTH_DEVICE_CREDENTIAL,
            Authenticators.BIOMETRIC_STRONG or Authenticators.DEVICE_CREDENTIAL,
            invalidatedByEnrollment = false,
        ),
        BIOMETRICS(
            "biometrics",
            KeyProperties.AUTH_BIOMETRIC_STRONG,
            Authenticators.BIOMETRIC_STRONG,
            invalidatedByEnrollment = false,
        ),
        CURRENT_BIOMETRICS(
            "currentBiometrics",
            KeyProperties.AUTH_BIOMETRIC_STRONG,
            Authenticators.BIOMETRIC_STRONG,
            invalidatedByEnrollment = true,
        ),
    }

    private companion object {
        const val KEYSTORE = "AndroidKeyStore"
        const val KEY_SIZE = 2048
        const val WRAPPED_KEY_SIZE = KEY_SIZE / 8
        const val DATA_KEY_SIZE = 256
        const val IV_SIZE = 12
        const val TAG_SIZE = 128
        const val WRAP = "RSA/ECB/OAEPWithSHA-256AndMGF1Padding"
        const val SEAL = "AES/GCM/NoPadding"

        // Keystore RSA keys default to SHA-1 for the OAEP MGF1 digest.
        val OAEP = OAEPParameterSpec("SHA-256", "MGF1", MGF1ParameterSpec.SHA1, PSource.PSpecified.DEFAULT)

        fun typeError(message: String) = TokamakPluginError("TypeError", message)

        fun notReadable() =
            TokamakPluginError("NotReadableError", "The stored value can no longer be decrypted")

        fun cancelled() = TokamakPluginError("NotAllowedError", "Authentication was cancelled")

        fun pluginError(error: Throwable): TokamakPluginError =
            when (error) {
                is TokamakPluginError -> error
                is KeyPermanentlyInvalidatedException,
                is BadPaddingException,
                is IllegalBlockSizeException,
                -> notReadable()
                is UserNotAuthenticatedException ->
                    TokamakPluginError("NotAllowedError", "The device must be unlocked to read this value")
                else -> TokamakPluginError("OperationError", error.message ?: error.javaClass.name)
            }
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
