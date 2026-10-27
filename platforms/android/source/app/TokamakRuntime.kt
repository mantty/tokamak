package com.tokamak.runtime

import java.io.File
import java.security.KeyStore

/** The tokamak runtime, running for the life of the process. */
internal class TokamakRuntime private constructor(private val handle: Long) {
    /**
     * The app's plugins, which the Worker calls. The runtime calls these methods on its own
     * threads, and each call or subscription is answered with [reply]: once for a call, and once
     * per value for a subscription.
     */
    interface Plugins {
        fun call(id: Long, plugin: String, method: String, payload: ByteArray)

        /** A call with a body, whose chunks the shell asks for with [pull] and receives in [chunk]. */
        fun callWithBody(id: Long, plugin: String, method: String, payload: ByteArray)

        /** The chunk of the call [id]'s body the shell pulled: bytes, or an encoded error when [failed]. */
        fun chunk(id: Long, payload: ByteArray, failed: Boolean, done: Boolean)

        fun subscribe(id: Long, plugin: String, method: String, payload: ByteArray)

        fun cancel(id: Long)
    }

    /** The loopback port the gateway bound. */
    val port: Int
        get() = nativePort(handle)

    /**
     * Delivers the event [name] with its encoded [payload] to the Worker's listeners, blocking
     * until they return or [timeoutMillis] passes, and returns their encoded reply, which is empty
     * when none replied.
     */
    fun emit(name: String, payload: ByteArray, timeoutMillis: Long): ByteArray =
        nativeEmit(handle, name, payload, timeoutMillis)

    /** What the app serves at [path], blocking until it arrives or [timeoutMillis] passes. */
    fun fetch(path: String, timeoutMillis: Long): ByteArray = nativeFetch(handle, path, timeoutMillis)

    /**
     * Records whether the app is in the foreground, which the Worker's `getLifecycleStage()`
     * reports from then on, and returns whether that changed.
     */
    fun setForeground(foreground: Boolean): Boolean = nativeSetForeground(handle, foreground)

    /** Whether the app is in the foreground, as last recorded. */
    val isForeground: Boolean
        get() = nativeIsForeground(handle)

    /** Asks the Worker for the next chunk of the body of the call [id]. */
    fun pull(id: Long) = nativePull(handle, id)

    /** Passes an encoded plugin result to its native invocation. */
    fun reply(id: Long, result: TokamakPluginResult) =
        nativeReply(handle, id, result.payload, result.failed, result.done)

    /**
     * The client certificate and PKCS#8 private key, both DER, to present for
     * [host]. Null when tokamak cannot authenticate the connection.
     */
    fun clientIdentity(host: String): Array<ByteArray>? = nativeClientIdentity(handle, host)

    companion object {
        init {
            System.loadLibrary("tokamak")
        }

        /**
         * Start the runtime, which delivers `start` with [foreground], whose Worker calls
         * [plugins], and whose gateway presents [server], throwing when it cannot serve the app.
         */
        fun start(
            packagedDir: File,
            stateDir: File,
            storageDir: File,
            host: String,
            server: KeyStore.PrivateKeyEntry,
            foreground: Boolean,
            plugins: Plugins,
        ): TokamakRuntime =
            TokamakRuntime(
                nativeStart(
                    packagedDir.path,
                    stateDir.path,
                    storageDir.path,
                    host,
                    server.chain(),
                    server.privateKey.encoded,
                    foreground,
                    plugins,
                ),
            )

        /**
         * Start the runtime against a host development server, delivering `start` with
         * [foreground], whose development Worker calls [plugins], and whose gateway presents
         * [server].
         */
        fun startDevelopment(
            stateDir: File,
            host: String,
            endpoint: String,
            sessionToken: String,
            server: KeyStore.PrivateKeyEntry,
            foreground: Boolean,
            plugins: Plugins,
        ): TokamakRuntime =
            TokamakRuntime(
                nativeStartDevelopment(
                    stateDir.path,
                    host,
                    endpoint,
                    sessionToken,
                    server.chain(),
                    server.privateKey.encoded,
                    foreground,
                    plugins,
                ),
            )

        /** The entry's certificates, leaf first, in DER. */
        private fun KeyStore.PrivateKeyEntry.chain(): Array<ByteArray> =
            certificateChain.map { it.encoded }.toTypedArray()

        @JvmStatic
        private external fun nativeStart(
            packagedDir: String,
            stateDir: String,
            storageDir: String,
            host: String,
            serverChain: Array<ByteArray>,
            serverKey: ByteArray,
            foreground: Boolean,
            plugins: Plugins,
        ): Long

        @JvmStatic
        private external fun nativeStartDevelopment(
            stateDir: String,
            host: String,
            endpoint: String,
            sessionToken: String,
            serverChain: Array<ByteArray>,
            serverKey: ByteArray,
            foreground: Boolean,
            plugins: Plugins,
        ): Long

        @JvmStatic
        private external fun nativeSetForeground(handle: Long, foreground: Boolean): Boolean

        @JvmStatic
        private external fun nativeIsForeground(handle: Long): Boolean

        @JvmStatic
        private external fun nativeReply(handle: Long, id: Long, payload: ByteArray, failed: Boolean, done: Boolean)

        @JvmStatic
        private external fun nativePull(handle: Long, id: Long)

        @JvmStatic
        private external fun nativePort(handle: Long): Int

        @JvmStatic
        private external fun nativeEmit(
            handle: Long,
            name: String,
            event: ByteArray,
            timeoutMillis: Long,
        ): ByteArray

        @JvmStatic
        private external fun nativeFetch(handle: Long, path: String, timeoutMillis: Long): ByteArray

        @JvmStatic
        private external fun nativeClientIdentity(handle: Long, host: String): Array<ByteArray>?
    }
}
