package com.tokamak.plugins.securestorage

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
import com.tokamak.runtime.TokamakHost
import com.tokamak.runtime.TokamakPlugin
import com.tokamak.runtime.TokamakPluginError
import com.tokamak.runtime.authenticateOwner
import com.tokamak.runtime.requireOwnerAuthentication
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
import kotlin.coroutines.resume
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withContext

/** When a stored value can be read. */
enum class Readable { WhenUnlocked, AfterFirstUnlock }

/** The authentication a read requires. */
enum class Authentication(val keyTypes: Int, val authenticators: Int, val invalidatedByEnrollment: Boolean) {
    BiometricsOrPasscode(
        KeyProperties.AUTH_BIOMETRIC_STRONG or KeyProperties.AUTH_DEVICE_CREDENTIAL,
        Authenticators.BIOMETRIC_STRONG or Authenticators.DEVICE_CREDENTIAL,
        invalidatedByEnrollment = false,
    ),
    Biometrics(KeyProperties.AUTH_BIOMETRIC_STRONG, Authenticators.BIOMETRIC_STRONG, invalidatedByEnrollment = false),
    CurrentBiometrics(KeyProperties.AUTH_BIOMETRIC_STRONG, Authenticators.BIOMETRIC_STRONG, invalidatedByEnrollment = true),
}

/** How a value is stored. */
data class StoreOptions(
    val readable: Readable,
    /** Reads need no authentication when null. */
    val authentication: Authentication?,
    /** Whether backups restore the value only to this device, which Android always does. */
    val thisDeviceOnly: Boolean,
    /** An Apple keychain feature, which Android rejects. */
    val keychainGroup: String?,
)

data class ReadOptions(
    /** The title of the system authentication prompt. */
    val prompt: String?,
    /** An Apple keychain feature, which Android rejects. */
    val keychainGroup: String?,
)

data class GroupOptions(
    /** An Apple keychain feature, which Android rejects. */
    val keychainGroup: String?,
)

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

    override fun executor(method: String) = worker

    fun create(name: String, value: String, options: StoreOptions) = translated { store(name, value, options, Write.CREATE) }

    fun replace(name: String, value: String, options: StoreOptions) = translated { store(name, value, options, Write.REPLACE) }

    fun set(name: String, value: String, options: StoreOptions) = translated { store(name, value, options, Write.SET) }

    /** Reads the value, asking the device owner for the authentication it was stored with. */
    suspend fun get(name: String, options: ReadOptions?): String {
        requireNoGroup(options?.keychainGroup)
        try {
            val stored = read(file(name)) ?: throw itemNotFound()
            val privateKey = keyStore.getKey(alias(stored.keyId), null) as? PrivateKey ?: throw notReadable()
            val unwrap = Cipher.getInstance(WRAP).apply { init(Cipher.DECRYPT_MODE, privateKey, OAEP) }
            val info = KeyFactory.getInstance(privateKey.algorithm, KEYSTORE).getKeySpec(privateKey, KeyInfo::class.java)
            if (info.isUserAuthenticationRequired) {
                val credential = (info.userAuthenticationType and KeyProperties.AUTH_DEVICE_CREDENTIAL) != 0
                val authenticators =
                    if (credential) Authenticators.BIOMETRIC_STRONG or Authenticators.DEVICE_CREDENTIAL
                    else Authenticators.BIOMETRIC_STRONG
                val title = options?.prompt?.takeIf { it.isNotEmpty() } ?: context.applicationInfo.loadLabel(context.packageManager)
                withContext(Dispatchers.Main) { authenticate(title, authenticators, unwrap) }
            }
            return open(stored, unwrap)
        } catch (error: Exception) {
            throw pluginError(error)
        }
    }

    fun delete(name: String, options: GroupOptions?) =
        translated {
            requireNoGroup(options?.keychainGroup)
            val file = file(name)
            val stored = read(file) ?: throw itemNotFound()
            AtomicFile(file).delete()
            keyStore.deleteEntry(alias(stored.keyId))
        }

    /** Deletes every value, and any key an interrupted write left behind. */
    fun clear(options: GroupOptions?) =
        translated {
            requireNoGroup(options?.keychainGroup)
            // An interrupted first write leaves only a `.new` file, which [read] ignores.
            val hadValues = directory.list().orEmpty().any { !it.endsWith(".new") }
            directory.deleteRecursively()
            keyStore.aliases().toList()
                .filter { it.startsWith(ALIAS_PREFIX) }
                .forEach(keyStore::deleteEntry)
            if (!hadValues) {
                throw TokamakPluginError("ItemNotFoundError", "No values are stored")
            }
        }

    /** Authorises [unwrap] with the device owner's authentication, in a prompt titled [title]. */
    private suspend fun authenticate(title: CharSequence, authenticators: Int, unwrap: Cipher) =
        suspendCancellableCoroutine { continuation ->
            val cancellation = CancellationSignal()
            continuation.invokeOnCancellation { cancellation.cancel() }
            host.authenticateOwner(title, authenticators, BiometricPrompt.CryptoObject(unwrap), { failed ->
                continuation.resumeWith(failed.map {})
            }, cancellation) { continuation.resume(Unit) }
        }

    private fun requireNoGroup(keychainGroup: String?) {
        if (keychainGroup != null) throw TokamakPluginError.notSupported("keychainGroup is an Apple keychain feature")
    }

    private fun store(name: String, value: String, options: StoreOptions, mode: Write) {
        requireNoGroup(options.keychainGroup)
        if (!options.thisDeviceOnly) {
            throw TokamakPluginError.notSupported("Android keeps secure storage values on this device")
        }
        val keyId = ByteArray(KEY_ID_SIZE).also(random::nextBytes)
        val spec =
            KeyGenParameterSpec.Builder(alias(keyId), KeyProperties.PURPOSE_DECRYPT)
                .setKeySize(KEY_SIZE)
                .setDigests(KeyProperties.DIGEST_SHA256)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_RSA_OAEP)
                .setUnlockedDeviceRequired(options.readable == Readable.WhenUnlocked)
        options.authentication?.let { requireAuthentication(spec, it) }
        val file = file(name)
        val previous = read(file)
        if (previous != null && mode == Write.CREATE) {
            throw TokamakPluginError("DuplicateItemError", "A value is already stored under this name")
        }
        if (previous == null && mode == Write.REPLACE) {
            throw itemNotFound()
        }
        val previousAlias = previous?.let { alias(it.keyId) }
        val publicKey = generatePublicKey(spec)
        try {
            write(file, seal(keyId, value, publicKey).encode())
        } catch (error: Throwable) {
            keyStore.deleteEntry(alias(keyId))
            throw error
        }
        previousAlias?.let(keyStore::deleteEntry)
    }

    private fun requireAuthentication(spec: KeyGenParameterSpec.Builder, authentication: Authentication) {
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

    private fun seal(keyId: ByteArray, value: String, publicKey: PublicKey): StoredValue {
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
        return StoredValue(keyId, wrapped, iv, sealed)
    }

    private fun open(stored: StoredValue, unwrap: Cipher): String {
        val dataKey = SecretKeySpec(unwrap.doFinal(stored.wrappedKey), "AES")
        return Cipher.getInstance(SEAL)
            .apply { init(Cipher.DECRYPT_MODE, dataKey, GCMParameterSpec(TAG_SIZE, stored.iv)) }
            .doFinal(stored.sealed)
            .decodeToString()
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

    /** Whether a write requires the name to have no value, a value, or either. */
    private enum class Write { CREATE, REPLACE, SET }

    /** Runs [operation], translating Keystore failures into the errors the Worker receives. */
    private fun <T> translated(operation: () -> T): T =
        try {
            operation()
        } catch (error: Exception) {
            throw pluginError(error)
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

        fun locked() = TokamakPluginError("NotAllowedError", "The device must be unlocked to read this value")

        fun notReadable() =
            TokamakPluginError("NotReadableError", "The stored value can no longer be decrypted")

        fun itemNotFound() = TokamakPluginError("ItemNotFoundError", "No value is stored under this name")

        fun pluginError(error: Exception): Exception =
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

/** A value's file: an unused string, then the key ID, wrapped data key, IV and sealed value. */
private class StoredValue(
    val keyId: ByteArray,
    val wrappedKey: ByteArray,
    val iv: ByteArray,
    val sealed: ByteArray,
) {
    fun encode(): ByteArray {
        val bytes = ByteArrayOutputStream()
        DataOutputStream(bytes).run {
            writeUTF("")
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
                readUTF()
                StoredValue(
                    keyId = readExactly(KEY_ID_SIZE),
                    wrappedKey = readExactly(WRAPPED_KEY_SIZE),
                    iv = readExactly(IV_SIZE),
                    sealed = readBytes(),
                )
            }

        private fun DataInputStream.readExactly(size: Int) = ByteArray(size).also(::readFully)
    }
}
