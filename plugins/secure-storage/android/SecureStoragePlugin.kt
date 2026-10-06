package com.tokamak.plugins.securestorage

import android.hardware.biometrics.BiometricManager.Authenticators
import android.hardware.biometrics.BiometricPrompt
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyInfo
import android.security.keystore.KeyPermanentlyInvalidatedException
import android.security.keystore.KeyProperties
import android.security.keystore.StrongBoxUnavailableException
import android.security.keystore.UserNotAuthenticatedException
import android.util.AtomicFile
import com.tokamak.runtime.TokamakHost
import com.tokamak.runtime.TokamakPlugin
import com.tokamak.runtime.TokamakPluginError
import com.tokamak.runtime.TokamakPluginReply
import com.tokamak.runtime.authenticateOwner
import com.tokamak.runtime.optionalString
import com.tokamak.runtime.requireBoolean
import com.tokamak.runtime.requireObject
import com.tokamak.runtime.requireOwnerAuthentication
import com.tokamak.runtime.requireString
import java.io.ByteArrayOutputStream
import java.io.DataInputStream
import java.io.DataOutputStream
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
 * carries the value's unlock and authentication requirements. Every write uses a new key
 * pair, identified in the value's file, so a failed write leaves the previous value readable.
 */
class TokamakSecureStoragePlugin(
    private val host: TokamakHost,
) : TokamakPlugin {
    override val id = "secure-storage"

    private val context = host.context
    private val worker = Executors.newSingleThreadExecutor()
    private val directory = File(context.noBackupFilesDir, "tokamak-secure-storage")
    private val keyStore = KeyStore.getInstance(KEYSTORE).apply { load(null) }
    private val random = SecureRandom()

    override fun call(method: String, arguments: Any?, reply: TokamakPluginReply) {
        when (method) {
            "set" -> execute(reply) {
                set(requireObject(arguments))
                reply(Result.success(null))
            }
            "get" -> execute(reply) { get(requireObject(arguments), reply) }
            "delete" -> execute(reply) {
                delete(requireObject(arguments).requireString("name"))
                reply(Result.success(null))
            }
            "keys" -> execute(reply) { reply(Result.success(keys())) }
            "clear" -> execute(reply) {
                clear()
                reply(Result.success(null))
            }
            else -> super.call(method, arguments, reply)
        }
    }

    private fun execute(reply: TokamakPluginReply, operation: () -> Unit) =
        worker.execute {
            runCatching(operation).onFailure { reply(Result.failure(pluginError(it))) }
        }

    private fun onUiThread(reply: TokamakPluginReply, operation: () -> Unit) =
        context.mainExecutor.execute {
            runCatching(operation).onFailure { reply(Result.failure(pluginError(it))) }
        }

    private fun set(request: JSONObject) {
        val name = request.requireString("name")
        val value = request.requireString("value")
        val unlockedDeviceRequired =
            when (request.requireString("readable")) {
                "whenUnlocked" -> true
                "afterFirstUnlock" -> false
                else -> throw TokamakPluginError.typeError("readable must be \"whenUnlocked\" or \"afterFirstUnlock\"")
            }
        if (!request.requireBoolean("thisDeviceOnly")) {
            throw TokamakPluginError.notSupported("Android keeps secure storage values on this device")
        }
        val keyId = ByteArray(KEY_ID_SIZE).also(random::nextBytes)
        val spec =
            KeyGenParameterSpec.Builder(alias(keyId), KeyProperties.PURPOSE_DECRYPT)
                .setKeySize(KEY_SIZE)
                .setDigests(KeyProperties.DIGEST_SHA256)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_RSA_OAEP)
                .setUnlockedDeviceRequired(unlockedDeviceRequired)
        request.optionalString("authentication")?.let { requireAuthentication(spec, it) }
        val file = file(name)
        val previousAlias = read(file)?.let { alias(it.keyId) }
        val publicKey = generatePublicKey(spec)
        try {
            write(file, seal(name, keyId, value, publicKey).encode())
        } catch (error: Throwable) {
            keyStore.deleteEntry(alias(keyId))
            throw error
        }
        previousAlias?.let(keyStore::deleteEntry)
    }

    private fun requireAuthentication(spec: KeyGenParameterSpec.Builder, wireName: String) {
        val authentication =
            Authentication.entries.firstOrNull { it.wireName == wireName } ?: throw unknownAuthentication()
        host.requireOwnerAuthentication(authentication.authenticators)
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

    private fun seal(name: String, keyId: ByteArray, value: String, publicKey: PublicKey): StoredValue {
        val dataKey = KeyGenerator.getInstance("AES").apply { init(DATA_KEY_SIZE) }.generateKey()
        val iv = ByteArray(IV_SIZE).also(random::nextBytes)
        val sealed =
            Cipher.getInstance(SEAL)
                .apply { init(Cipher.ENCRYPT_MODE, dataKey, GCMParameterSpec(TAG_SIZE, iv)) }
                .doFinal(value.toByteArray())
        val wrapped =
            Cipher.getInstance(WRAP)
                .apply { init(Cipher.ENCRYPT_MODE, publicKey, OAEP) }
                .doFinal(dataKey.encoded)
        return StoredValue(name, keyId, wrapped, iv, sealed)
    }

    private fun get(request: JSONObject, reply: TokamakPluginReply) {
        val name = request.requireString("name")
        val prompt = request.optionalString("prompt")
        val stored = read(file(name))
        if (stored == null) {
            reply(Result.success(null))
            return
        }
        val privateKey =
            keyStore.getKey(alias(stored.keyId), null) as? PrivateKey
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
        authenticate(authenticators, prompt, unwrap, stored, reply)
    }

    /** Authorises [unwrap] with the device owner's authentication, then replies with the opened value. */
    private fun authenticate(
        authenticators: Int,
        prompt: String?,
        unwrap: Cipher,
        stored: StoredValue,
        reply: TokamakPluginReply,
    ) {
        val title = prompt?.takeIf { it.isNotEmpty() } ?: context.applicationInfo.loadLabel(context.packageManager)
        onUiThread(reply) {
            host.authenticateOwner(title, authenticators, BiometricPrompt.CryptoObject(unwrap), reply) {
                execute(reply) { reply(Result.success(open(stored, unwrap))) }
            }
        }
    }

    private fun open(stored: StoredValue, unwrap: Cipher): String {
        val dataKey = SecretKeySpec(unwrap.doFinal(stored.wrappedKey), "AES")
        return Cipher.getInstance(SEAL)
            .apply { init(Cipher.DECRYPT_MODE, dataKey, GCMParameterSpec(TAG_SIZE, stored.iv)) }
            .doFinal(stored.sealed)
            .decodeToString()
    }

    private fun delete(name: String) {
        val file = file(name)
        val stored = read(file) ?: return
        AtomicFile(file).delete()
        keyStore.deleteEntry(alias(stored.keyId))
    }

    /** AtomicFile's in-progress and backup files carry a suffix; value files have none. */
    private fun keys(): List<String> =
        directory.listFiles { file -> '.' !in file.name }
            .orEmpty()
            .mapNotNull { read(it)?.name }
            .sorted()

    private fun clear() {
        directory.deleteRecursively()
        keyStore.aliases().toList()
            .filter { it.startsWith(ALIAS_PREFIX) }
            .forEach(keyStore::deleteEntry)
    }

    private fun read(file: File): StoredValue? =
        try {
            StoredValue.decode(AtomicFile(file).readFully())
        } catch (_: FileNotFoundException) {
            null
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

    private fun file(name: String) =
        File(directory, MessageDigest.getInstance("SHA-256").digest(name.toByteArray()).toHex())

    private fun alias(keyId: ByteArray) = ALIAS_PREFIX + keyId.toHex()

    private fun ByteArray.toHex() = joinToString("") { "%02x".format(it) }

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
        const val ALIAS_PREFIX = "tokamak-secure-storage-"
        const val DATA_KEY_SIZE = 256
        const val TAG_SIZE = 128
        const val WRAP = "RSA/ECB/OAEPWithSHA-256AndMGF1Padding"
        const val SEAL = "AES/GCM/NoPadding"

        // Keystore RSA keys default to SHA-1 for the OAEP MGF1 digest.
        val OAEP = OAEPParameterSpec("SHA-256", "MGF1", MGF1ParameterSpec.SHA1, PSource.PSpecified.DEFAULT)

        fun unknownAuthentication() =
            TokamakPluginError.typeError("authentication must be \"biometricsOrPasscode\", \"biometrics\" or \"currentBiometrics\"")

        fun locked() = TokamakPluginError("NotAllowedError", "The device must be unlocked to read this value")

        fun notReadable() =
            TokamakPluginError("NotReadableError", "The stored value can no longer be decrypted")

        fun pluginError(error: Throwable): Throwable =
            when (error) {
                is KeyPermanentlyInvalidatedException,
                is BadPaddingException,
                is IllegalBlockSizeException,
                -> notReadable()
                is UserNotAuthenticatedException -> locked()
                else -> error
            }
    }
}

private const val KEY_SIZE = 2048
private const val KEY_ID_SIZE = 16
private const val WRAPPED_KEY_SIZE = KEY_SIZE / 8
private const val IV_SIZE = 12

/** A value's file: its name, key ID, wrapped data key, IV, then sealed value. */
private class StoredValue(
    val name: String,
    val keyId: ByteArray,
    val wrappedKey: ByteArray,
    val iv: ByteArray,
    val sealed: ByteArray,
) {
    fun encode(): ByteArray {
        val bytes = ByteArrayOutputStream()
        DataOutputStream(bytes).run {
            writeUTF(name)
            write(keyId)
            write(wrappedKey)
            write(iv)
            write(sealed)
        }
        return bytes.toByteArray()
    }

    companion object {
        fun decode(bytes: ByteArray): StoredValue =
            DataInputStream(bytes.inputStream()).run {
                StoredValue(
                    name = readUTF(),
                    keyId = readExactly(KEY_ID_SIZE),
                    wrappedKey = readExactly(WRAPPED_KEY_SIZE),
                    iv = readExactly(IV_SIZE),
                    sealed = readBytes(),
                )
            }

        private fun DataInputStream.readExactly(size: Int) = ByteArray(size).also(::readFully)
    }
}
