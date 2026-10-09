package com.tokamak.runtime

import java.io.File

/** The tokamak runtime, running for the life of the process. */
internal class TokamakRuntime private constructor(private val handle: Long) {
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
         * Start the runtime, which delivers `start` with [foreground], throwing when it cannot
         * serve the app.
         */
        fun start(
            packagedDir: File,
            stateDir: File,
            storageDir: File,
            host: String,
            foreground: Boolean,
        ): TokamakRuntime =
            TokamakRuntime(
                nativeStart(packagedDir.path, stateDir.path, storageDir.path, host, foreground),
            )

        /** Start the runtime against a host development server, delivering `start` with [foreground]. */
        fun startDevelopment(
            stateDir: File,
            host: String,
            endpoint: String,
            sessionToken: String,
            foreground: Boolean,
        ): TokamakRuntime =
            TokamakRuntime(
                nativeStartDevelopment(stateDir.path, host, endpoint, sessionToken, foreground),
            )

        @JvmStatic
        private external fun nativeStart(
            packagedDir: String,
            stateDir: String,
            storageDir: String,
            host: String,
            foreground: Boolean,
        ): Long

        @JvmStatic
        private external fun nativeStartDevelopment(
            stateDir: String,
            host: String,
            endpoint: String,
            sessionToken: String,
            foreground: Boolean,
        ): Long

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
        private external fun nativeServerAuthority(handle: Long, host: String): ByteArray?

        @JvmStatic
        private external fun nativeClientIdentity(
            handle: Long,
            host: String,
            previousFailures: Int,
        ): Array<ByteArray>?
    }
}
