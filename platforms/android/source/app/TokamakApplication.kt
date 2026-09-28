package com.tokamak.runtime

import android.app.Activity
import android.app.Application
import android.content.Context
import android.content.pm.PackageManager
import java.io.File

private const val HOST_METADATA = "tokamak.host"
private const val DEV_ENDPOINT_METADATA = "tokamak.dev.endpoint"
private const val DEV_SESSION_TOKEN_METADATA = "tokamak.dev.session-token"

/**
 * The tokamak process. Owns the runtime and native plugins, which outlive any activity, so a
 * plugin can run Worker code when the system starts the app in the background.
 */
class TokamakApplication : Application(), TokamakHost {
    override val context: Context
        get() = this

    @Volatile
    override var activity: Activity? = null

    /** Every plugin, by ID. */
    val plugins: Map<String, TokamakPlugin> by lazy {
        buildMap {
            tokamakPlugins(this@TokamakApplication).forEach { plugin ->
                check(put(plugin.id, plugin) == null) { "Duplicate plugin ID" }
            }
        }
    }

    /** The running runtime, started on first use. Blocks while it starts. */
    internal val runtime: TokamakRuntime by lazy(::startRuntime)

    /** The host of the app origin, `https://<appHost>`. */
    val appHost: String by lazy {
        requireNotNull(metadata()?.getString(HOST_METADATA)) { "$HOST_METADATA is required" }
    }

    override fun dispatch(event: String, payload: String, timeoutMillis: Long): String? =
        runtime.dispatch(event, payload, timeoutMillis)

    override fun plugin(id: String): TokamakPlugin? = plugins[id]

    private fun startRuntime(): TokamakRuntime {
        val metadata = metadata()
        val endpoint = metadata?.getString(DEV_ENDPOINT_METADATA)
        val sessionToken = metadata?.getString(DEV_SESSION_TOKEN_METADATA)
        return if (!endpoint.isNullOrEmpty() && !sessionToken.isNullOrEmpty()) {
            TokamakRuntime.startDevelopment(stateDir(), appHost, endpoint, sessionToken)
        } else {
            TokamakRuntime.start(unpackApp(), stateDir(), appHost)
        }
    }

    private fun metadata() =
        packageManager.getApplicationInfo(packageName, PackageManager.GET_META_DATA).metaData

    /** Copy the packaged app out of the APK once per install, so the runtime can read it as files. */
    private fun unpackApp(): File {
        val app = File(filesDir, "tokamak/app")
        val unpacked = File(filesDir, "tokamak/app.installed")
        val installed = packageManager.getPackageInfo(packageName, 0).lastUpdateTime.toString()
        if (unpacked.isFile && unpacked.readText() == installed) return app
        app.deleteRecursively()
        copyAsset("app", app)
        unpacked.writeText(installed)
        return app
    }

    private fun copyAsset(source: String, destination: File) {
        val entries = assets.list(source) ?: emptyArray()
        if (entries.isEmpty()) {
            destination.parentFile?.mkdirs()
            assets.open(source).use { input ->
                destination.outputStream().use(input::copyTo)
            }
            return
        }
        destination.mkdirs()
        for (entry in entries) copyAsset("$source/$entry", File(destination, entry))
    }

    private fun stateDir(): File = File(filesDir, "tokamak/state").apply { mkdirs() }
}
