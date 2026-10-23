package com.tokamak.plugins.example

import com.tokamak.runtime.TokamakHost
import com.tokamak.runtime.TokamakPlugin
import com.tokamak.runtime.TokamakPluginBody
import com.tokamak.runtime.TokamakPluginError
import com.tokamak.runtime.TokamakReference
import com.tokamak.runtime.TokamakTag
import java.util.concurrent.Executor
import java.util.concurrent.Executors
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.flow
import kotlinx.coroutines.flow.flowOf
import kotlinx.coroutines.suspendCancellableCoroutine
import org.json.JSONObject

enum class Precision { precise, approximate }

data class Region(
    val id: String,
    val radius: Double,
    val onEntry: Boolean?,
    val tags: List<String?>,
    val counts: Map<String, Int>,
    val data: JSONObject?,
    val kind: String? = "region",
)

data class Visit(val place: String)

/** How a region alerts: a name, a value, or an object with a field only its form has. */
sealed interface Alert {
    data object Silent : Alert

    data object Muted : Alert

    @JvmInline value class Volume(val value: Double) : Alert

    @JvmInline value class Pattern(val value: List<Int>) : Alert

    @JvmInline value class Enabled(val value: Boolean) : Alert

    data class Tone(val frequency: Double, val volume: Double?) : Alert

    data class Flash(val colour: String, val volume: Double?) : Alert
}

/** A region's mark: none, or labels, an object of any keys. */
sealed interface Mark {
    data object None : Mark

    @JvmInline value class Labels(val value: Map<String, String>) : Mark
}

/** Where a region alerts, tagged by its kind. */
@TokamakTag("kind")
sealed interface Zone {
    data object Everywhere : Zone

    data class Circle(val radius: Double) : Zone
}

/** Every kind of method the Worker calls, for the describer's tests. */
class TokamakExamplePlugin(private val host: TokamakHost) : TokamakPlugin {
    override val id = "example"
    private val added = mutableListOf<Region>()
    private val executor = Executors.newSingleThreadExecutor()

    override fun executor(method: String): Executor = executor

    suspend fun addRegion(region: Region, replace: Boolean?) {
        added += region
    }

    fun regions(): List<Region> = added

    fun permission(precision: Precision): Map<String, String?> = mapOf("precision" to precision.name)

    fun watchPosition(every: Double): Flow<Region> = flowOf(Region("$every", 1.0, null, emptyList(), emptyMap(), null))

    fun onVisit(): Flow<String> = flow { throw TokamakPluginError("NotAllowedError", "No visits") }

    suspend fun capture(quality: Int): ByteArray = ByteArray(2) { quality.toByte() }

    fun encode(frame: TokamakReference<ByteArray>): Int = frame.value.size

    suspend fun upload(name: String, file: TokamakPluginBody): Int =
        suspendCancellableCoroutine { continuation ->
            file.read { chunk ->
                chunk.fold({ continuation.resume(it?.size ?: 0) }, { continuation.resumeWithException(it) })
            }
        }

    fun digest(data: ByteArray): String = data.joinToString("") { "%02x".format(it) }

    fun read(path: String): ByteArray {
        if (path != "a.txt") throw TokamakPluginError("NotFoundError", path)
        return byteArrayOf(9)
    }

    fun copy(data: ByteArray): ByteArray = data

    fun mirror(value: Any): Any = value

    fun alert(alert: Alert, zone: Zone): List<Alert> = if (zone == Zone.Everywhere) listOf(alert, Alert.Silent) else listOf(alert)

    fun zone(zone: Zone): Zone = zone

    fun mark(mark: Mark): Mark = mark
}
