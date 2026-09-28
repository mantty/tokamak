package com.tokamak.runtime

import java.io.File

/** The tokamak runtime, running for the life of the process. */
internal class TokamakRuntime private constructor(private val handle: Long) {
    /** The loopback port the gateway bound. */
    val port: Int
        get() = nativePort(handle)

    fun restoreGateway(): Int = nativeRestoreGateway(handle)

    fun suspend() = nativeSuspend(handle)

    fun resume() = nativeResume(handle)

    /**
     * Runs the Worker's [event] handler with a JSON [payload], blocking until it settles.
     * Returns the handler's JSON result, or null when the Worker has no such handler.
     */
    fun dispatch(event: String, payload: String): String? = nativeDispatch(handle, event, payload)

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

        /** Start the runtime, throwing when it cannot serve the app. */
        fun start(packagedDir: File, stateDir: File, host: String): TokamakRuntime =
            TokamakRuntime(nativeStart(packagedDir.path, stateDir.path, host))

        /** Start the runtime against a host development server. */
        fun startDevelopment(
            stateDir: File,
            host: String,
            endpoint: String,
            sessionToken: String,
        ): TokamakRuntime =
            TokamakRuntime(nativeStartDevelopment(stateDir.path, host, endpoint, sessionToken))

        @JvmStatic
        private external fun nativeStart(packagedDir: String, stateDir: String, host: String): Long

        @JvmStatic
        private external fun nativeStartDevelopment(
            stateDir: String,
            host: String,
            endpoint: String,
            sessionToken: String,
        ): Long

        @JvmStatic
        private external fun nativePort(handle: Long): Int

        @JvmStatic
        private external fun nativeRestoreGateway(handle: Long): Int

        @JvmStatic
        private external fun nativeSuspend(handle: Long)

        @JvmStatic
        private external fun nativeResume(handle: Long)

        @JvmStatic
        private external fun nativeDispatch(handle: Long, event: String, payload: String): String?

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
