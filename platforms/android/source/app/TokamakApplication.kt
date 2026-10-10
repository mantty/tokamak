package com.tokamak.runtime

import android.app.Activity
import android.app.Application
import android.content.Context
import android.content.pm.PackageManager
import android.os.SystemClock
import android.util.Log
import androidx.lifecycle.DefaultLifecycleObserver
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.ProcessLifecycleOwner
import java.io.File
import java.util.concurrent.Executors
import org.json.JSONTokener

private const val HOST_METADATA = "tokamak.host"
private const val DEV_ENDPOINT_METADATA = "tokamak.dev.endpoint"
private const val DEV_SESSION_TOKEN_METADATA = "tokamak.dev.session-token"

/** How long the Worker has for `resume` and `suspend`. */
private const val LIFECYCLE_DEADLINE_MILLIS = 10_000L

/**
 * The tokamak process. Owns the runtime and native plugins, which outlive any activity, so a
 * plugin can run Worker code when the system starts the app in the background.
 */
class TokamakApplication : Application(), TokamakPlugins {
    /** The app's activity, or null while it has none. */
    @Volatile
    var activity: Activity? = null

    /** Every plugin, by ID. */
    val plugins: Map<String, TokamakPlugin> by lazy {
        buildMap {
            tokamakPlugins(this@TokamakApplication).forEach { plugin ->
                check(put(plugin.id, plugin) == null) { "Duplicate plugin ID" }
            }
        }
    }

    internal val permissionRequests = TokamakPermissions(this)

    private val started = lazy(::startRuntime)

    /** The running runtime, started on first use. Blocks while it starts. */
    internal val runtime: TokamakRuntime by started

    /** The Worker's calls and subscriptions on the plugins. Main thread only. */
    private val workerRequests by lazy {
        TokamakPluginRequests<Long>(plugins) { id, result -> runtime.reply(id, result.toString()) }
    }

    /** The plugins as the runtime calls them, on its own threads. */
    private val workerPlugins =
        object : TokamakRuntime.Plugins {
            override fun call(id: Long, plugin: String, method: String, arguments: String) =
                mainExecutor.execute { workerRequests.run(id, false, plugin, method, parse(arguments)) }

            override fun subscribe(id: Long, plugin: String, method: String, arguments: String) =
                mainExecutor.execute { workerRequests.run(id, true, plugin, method, parse(arguments)) }

            override fun unsubscribe(id: Long) = mainExecutor.execute { workerRequests.cancel(id) }

            private fun parse(arguments: String): Any? = JSONTokener(arguments).nextValue()
        }

    /** The host of the app origin, `https://<appHost>`. */
    val appHost: String by lazy {
        requireNotNull(metadata()?.getString(HOST_METADATA)) { "$HOST_METADATA is required" }
    }

    /** Delivers `resume` and `suspend` one at a time, in order. */
    private val lifecycle = Executors.newSingleThreadExecutor()

    override fun onCreate() {
        super.onCreate()
        ProcessLifecycleOwner.get().lifecycle.addObserver(
            object : DefaultLifecycleObserver {
                override fun onStart(owner: LifecycleOwner) = moved(foreground = true)

                override fun onStop(owner: LifecycleOwner) = moved(foreground = false)
            },
        )
    }

    override fun plugin(id: String): TokamakPlugin? = plugins[id]

    /** The host the plugin [id] sees, whose events the Worker's listeners receive as `<id>.<name>`. */
    internal fun host(id: String): TokamakHost =
        object : TokamakHost {
            override val context: Context
                get() = this@TokamakApplication

            override val activity: Activity?
                get() = this@TokamakApplication.activity

            override val isInForeground: Boolean
                get() = this@TokamakApplication.isInForeground

            override fun requireForegroundActivity(): Activity =
                this@TokamakApplication.requireForegroundActivity()

            override fun wouldPrompt(permission: String): Boolean =
                permissionRequests.wouldPrompt(permission)

            override fun requestPermissions(
                permissions: Set<String>,
                callback: (Map<String, Boolean>) -> Unit,
            ) = permissionRequests.request(permissions, callback)

            override fun emit(name: String, event: String, timeoutMillis: Long): String =
                emitEvent("$id.$name", event, timeoutMillis)
        }

    /** The activity to show UI in; throws `NeedsUIError` while the app is in the background. */
    internal fun requireForegroundActivity(): Activity =
        activity?.takeIf { isInForeground } ?: throw TokamakPluginError.needsUI()

    /**
     * Whether the app is in the foreground, as the runtime records it, or until the runtime has
     * started, whether an activity exists to start it in the foreground.
     */
    private val isInForeground: Boolean
        get() = if (started.isInitialized()) runtime.isForeground else activity != null

    /**
     * Delivers the event [name] with the JSON [event] to the Worker's listeners and returns their
     * reply as JSON. Starts the runtime when it is not running. Blocks.
     */
    private fun emitEvent(name: String, event: String, timeoutMillis: Long): String {
        val deadline = SystemClock.elapsedRealtime() + timeoutMillis
        val started = runtime
        return started.emit(name, event, deadline - SystemClock.elapsedRealtime())
    }

    /** Records the app moving into or out of the foreground, once the runtime has started. */
    private fun moved(foreground: Boolean) {
        if (started.isInitialized()) record(runtime, foreground)
    }

    /** Records [foreground] in [runtime], delivering `resume` or `suspend` when it changed. */
    private fun record(runtime: TokamakRuntime, foreground: Boolean) {
        if (!runtime.setForeground(foreground)) return
        val name = if (foreground) "resume" else "suspend"
        lifecycle.execute {
            runCatching { runtime.emit(name, "{}", LIFECYCLE_DEADLINE_MILLIS) }
                .onFailure { Log.w("tokamak", "tokamak $name failed", it) }
        }
    }

    /**
     * Starts the runtime, which delivers `start` in the foreground when an activity started it,
     * then records where the app has moved since.
     */
    private fun startRuntime(): TokamakRuntime {
        val metadata = metadata()
        val endpoint = metadata?.getString(DEV_ENDPOINT_METADATA)
        val sessionToken = metadata?.getString(DEV_SESSION_TOKEN_METADATA)
        val startsInForeground = activity != null
        val runtime =
            if (!endpoint.isNullOrEmpty() && !sessionToken.isNullOrEmpty()) {
                TokamakRuntime.startDevelopment(
                    stateDir(), appHost, endpoint, sessionToken, startsInForeground, workerPlugins,
                )
            } else {
                TokamakRuntime.start(unpackApp(), stateDir(), storageDir(), appHost, startsInForeground, workerPlugins)
            }
        mainExecutor.execute {
            record(runtime, ProcessLifecycleOwner.get().lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED))
        }
        return runtime
    }

    private fun metadata() =
        packageManager.getApplicationInfo(packageName, PackageManager.GET_META_DATA).metaData

    /** Copy the packaged app out of the APK once per install, so the runtime can read it as files. */
    private fun unpackApp(): File {
        val app = File(noBackupFilesDir, "tokamak/app")
        val unpacked = File(noBackupFilesDir, "tokamak/app.installed")
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

    /** Runtime state, which backups leave out. */
    private fun stateDir(): File = File(noBackupFilesDir, "tokamak/state").apply { mkdirs() }

    /**
     * Stores behind storage bindings, which Auto Backup includes. tokamak keeps nothing else in the
     * files directory, so everything else under `tokamak` there is removed.
     */
    private fun storageDir(): File {
        val root = File(filesDir, "tokamak")
        root.listFiles()?.filter { it.name != "storage" }?.forEach { it.deleteRecursively() }
        return File(root, "storage").apply { mkdirs() }
    }
}
