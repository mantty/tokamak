package com.tokamak.runtime

import java.io.File

/** The tokamak runtime, running for the life of the process. */
internal class TokamakRuntime private constructor(private val handle: Long) {
    /**
     * The app's plugins, which the Worker calls. The runtime calls these methods on its own
     * threads, and each call or subscription is answered with [reply]: once for a call, and once
     * per value for a subscription.
     */
    interface Plugins {
        fun call(id: Long, plugin: String, method: String, arguments: String)

        fun subscribe(id: Long, plugin: String, method: String, arguments: String)

        fun unsubscribe(id: Long)
    }

    /** The loopback port the gateway bound. */
    val port: Int
        get() = nativePort(handle)

    fun restoreGateway(): Int = nativeRestoreGateway(handle)

    /**
     * Delivers the event [name] with the JSON [event] to the Worker's listeners, blocking until
     * they return or [timeoutMillis] passes, and returns their reply as JSON.
     */
    fun emit(name: String, event: String, timeoutMillis: Long): String =
        nativeEmit(handle, name, event, timeoutMillis)

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

    /** Passes a plugin's JSON [result] to the Worker's call or subscription [id]. */
    fun reply(id: Long, result: String) = nativeReply(handle, id, result)

    /**
     * The authority a server certificate for [host] must chain to, or null
     * when tokamak does not vouch for the host.
     */
    fun serverAuthority(host: String): ByteArray? = nativeServerAuthority(handle, host)

    /**
     * The client certificate and PKCS#8 private key, both DER, to present for
     * [host]. Null when tokamak cannot authenticate the connection.
     */
    fun clientIdentity(host: String, previousFailures: Int): Array<ByteArray>? =
        nativeClientIdentity(handle, host, previousFailures)

    companion object {
        init {
            System.loadLibrary("tokamak")
        }

        /**
         * Start the runtime, which delivers `start` with [foreground] and whose Worker calls
         * [plugins], throwing when it cannot serve the app.
         */
        fun start(
            packagedDir: File,
            stateDir: File,
            storageDir: File,
            host: String,
            foreground: Boolean,
            plugins: Plugins,
        ): TokamakRuntime =
            TokamakRuntime(
                nativeStart(packagedDir.path, stateDir.path, storageDir.path, host, foreground, plugins),
            )

        /**
         * Start the runtime against a host development server, delivering `start` with
         * [foreground], whose development Worker calls [plugins].
         */
        fun startDevelopment(
            stateDir: File,
            host: String,
            endpoint: String,
            sessionToken: String,
            foreground: Boolean,
            plugins: Plugins,
        ): TokamakRuntime =
            TokamakRuntime(
                nativeStartDevelopment(stateDir.path, host, endpoint, sessionToken, foreground, plugins),
            )

        @JvmStatic
        private external fun nativeStart(
            packagedDir: String,
            stateDir: String,
            storageDir: String,
            host: String,
            foreground: Boolean,
            plugins: Plugins,
        ): Long

        @JvmStatic
        private external fun nativeStartDevelopment(
            stateDir: String,
            host: String,
            endpoint: String,
            sessionToken: String,
            foreground: Boolean,
            plugins: Plugins,
        ): Long

        @JvmStatic
        private external fun nativeSetForeground(handle: Long, foreground: Boolean): Boolean

        @JvmStatic
        private external fun nativeIsForeground(handle: Long): Boolean

        @JvmStatic
        private external fun nativeReply(handle: Long, id: Long, result: String)

        @JvmStatic
        private external fun nativePort(handle: Long): Int

        @JvmStatic
        private external fun nativeRestoreGateway(handle: Long): Int

        @JvmStatic
        private external fun nativeEmit(
            handle: Long,
            name: String,
            event: String,
            timeoutMillis: Long,
        ): String

        @JvmStatic
        private external fun nativeFetch(handle: Long, path: String, timeoutMillis: Long): ByteArray

        @JvmStatic
        private external fun nativeServerAuthority(handle: Long, host: String): ByteArray?

        @JvmStatic
        private external fun nativeClientIdentity(
            handle: Long,
            host: String,
            previousFailures: Int,
        ): Array<ByteArray>?
    }
}
