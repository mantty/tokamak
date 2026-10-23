package com.tokamak.plugins.example

import android.app.Activity
import android.content.Context
import com.tokamak.runtime.TokamakDispatch
import com.tokamak.runtime.TokamakHost
import com.tokamak.runtime.TokamakListeners
import com.tokamak.runtime.TokamakPlugin
import com.tokamak.runtime.TokamakPluginBytesReply
import com.tokamak.runtime.TokamakPluginError
import com.tokamak.runtime.TokamakPluginRequest
import com.tokamak.runtime.TokamakPluginRequests
import com.tokamak.runtime.TokamakPluginResult
import com.tokamak.runtime.TokamakResources
import com.tokamak.runtime.TokamakValue
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executor
import java.util.concurrent.TimeUnit
import kotlin.math.pow
import kotlin.reflect.KClass
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.flow
import kotlinx.coroutines.flow.take
import kotlinx.coroutines.flow.toList
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull
import kotlinx.coroutines.yield
import org.json.JSONArray
import org.json.JSONObject
import org.junit.Test

/** Runs the Worker's requests through the fixture plugin's generated dispatch. */
class DispatchTest {
    private val replies = mutableMapOf<Int, MutableList<TokamakPluginResult>>()
    private val pulled = CountDownLatch(1)
    private val requests = TokamakPluginRequests<Int>(
        mapOf("example" to TokamakExamplePluginDispatch(Host())),
        pull = { pulled.countDown() },
    ) { id, result -> synchronized(replies) { replies.getOrPut(id) { mutableListOf() } += result } }
    private var next = 0

    private fun request(method: String, payload: String, kind: TokamakPluginRequest = TokamakPluginRequest.CALL): Int {
        requests.run(++next, kind, "example", method, payload.toByteArray())
        return next
    }

    /** The first [count] replies to [id], once they arrive. */
    private fun wait(id: Int, count: Int = 1): List<TokamakPluginResult> {
        val deadline = System.nanoTime() + 2_000_000_000
        while (synchronized(replies) { replies[id].orEmpty().size } < count && System.nanoTime() < deadline) Thread.sleep(1)
        return synchronized(replies) { replies[id].orEmpty().toList() }.also { check(it.size >= count) { "request $id has ${it.size} replies" } }
    }

    private fun reply(id: Int) = wait(id).last()

    private fun TokamakPluginResult.text() = payload.toString(Charsets.UTF_8)

    @Test
    fun dispatch() {
        val region = """{"id":"home","radius":50,"tags":["a",null],"counts":{"visits":2},"data":{"n":[1,true,null]}}"""
        val added = reply(request("addRegion", """{"region":$region}"""))
        check(!added.failed && added.done && added.text() == "null") { added.text() }
        val regions = JSONArray(reply(request("regions", "{}")).text())
        check(regions.getJSONObject(0).getString("id") == "home" && regions.getJSONObject(0).getJSONArray("tags").isNull(1))
        // A nil field is absent, as Swift's Codable encodes it.
        check(!regions.getJSONObject(0).has("onEntry")) { regions.toString() }
        val data = regions.getJSONObject(0).getJSONObject("data").getJSONArray("n")
        check(data.getInt(0) == 1 && data.getBoolean(1) && data.isNull(2)) { data.toString() }

        val invalid = reply(request("addRegion", """{"region":{"id":1}}"""))
        check(invalid.failed && invalid.text().contains("region.id must be a string")) { invalid.text() }
        val fraction = reply(request("capture", """{"quality":1.5}""", TokamakPluginRequest.SUBSCRIBE))
        check(fraction.failed && fraction.text().contains("quality must be a whole number")) { fraction.text() }
        val precision = reply(request("permission", """{"precision":"exact"}"""))
        check(precision.failed && precision.text().contains("precise, approximate")) { precision.text() }
        check(JSONObject(reply(request("permission", """{"precision":"precise"}""")).text()).getString("precision") == "precise")

        check(reply(request("read", """{"path":"a.txt"}""")).payload.contentEquals(byteArrayOf(9)))
        check(reply(request("read", """{"path":"b.txt"}""")).text().contains("NotFoundError"))
        check(reply(request("digest", "\u0001\u0002")).text() == "\"0102\"")
        check(reply(request("copy", "\u0003")).payload.contentEquals(byteArrayOf(3)))
        val mirrored = reply(request("mirror", """{"value":{"a":[1,null]}}""")).text()
        check(mirrored == """{"a":[1,null]}""") { mirrored }
        val missing = reply(request("mirror", """{"value":null}"""))
        check(missing.failed && missing.text().contains("value must be a value other than null")) { missing.text() }

        // A sealed type's form is chosen by its JSON, and encodes as it decodes.
        val circle = """{"kind":"circle","radius":2}"""
        val alerts = listOf(
            "\"muted\"" to """["muted","silent"]""", "0.5" to "[0.5]", "[1,2]" to "[[1,2]]", "true" to "[true]",
            """{"frequency":440}""" to """[{"frequency":440}]""", """{"colour":"red","volume":1}""" to """[{"colour":"red","volume":1}]""",
        )
        for ((alert, encoded) in alerts) {
            val zone = if (alert == "\"muted\"") """{"kind":"everywhere"}""" else circle
            val alerted = reply(request("alert", """{"alert":$alert,"zone":$zone}"""))
            check(JSONArray(alerted.text()).similar(JSONArray(encoded))) { alerted.text() }
        }
        check(JSONObject(reply(request("zone", """{"zone":$circle}""")).text()).similar(JSONObject(circle)))
        check(reply(request("mark", """{"mark":"none"}""")).text() == "\"none\"")
        check(JSONObject(reply(request("mark", """{"mark":{"a":"b"}}""")).text()).similar(JSONObject("""{"a":"b"}""")))
        val forms = listOf(
            Triple("\"loud\"", circle, "alert must be one of Alert's forms"),
            Triple("""{"volume":1}""", circle, "alert must be one of Alert's forms"),
            Triple("1", """{"kind":"square"}""", "zone must be one of Zone's forms"),
            Triple("1", """{"radius":2}""", "zone.kind must be a string"),
        )
        for ((alert, zone, problem) in forms) {
            val rejected = reply(request("alert", """{"alert":$alert,"zone":$zone}"""))
            check(rejected.failed && rejected.text().contains(problem)) { rejected.text() }
        }

        val positions = wait(request("watchPosition", """{"every":5}""", TokamakPluginRequest.SUBSCRIBE), 2)
        check(positions[0].text().contains(""""id":"5.0"""") && !positions[0].done)
        check(positions[1].payload.isEmpty() && positions[1].done)
        val visits = reply(request("onVisit", "{}", TokamakPluginRequest.SUBSCRIBE))
        check(visits.failed && visits.done && visits.text().contains("NotAllowedError"))

        val frame = request("capture", """{"quality":7}""", TokamakPluginRequest.SUBSCRIBE)
        val id = reply(frame).text()
        check(reply(request("encode", """{"frame":$id}""")).text() == "2")
        requests.cancel(frame)
        Thread.sleep(50)
        val released = reply(request("encode", """{"frame":$id}"""))
        check(released.text().contains("The native reference is closed")) { released.text() }

        val upload = request("upload", """{"name":"a.txt"}""", TokamakPluginRequest.CALL_WITH_BODY)
        pulled.await()
        requests.chunk(upload, TokamakPluginResult(byteArrayOf(1, 2, 3), false, false))
        check(reply(upload).text() == "3")
    }

    @Test
    fun numbers() {
        val long = runCatching { TokamakValue(2.0.pow(63), "n").long() }.exceptionOrNull()
        check(long?.message == "n must be a Long") { "$long" }
        val int = runCatching { TokamakValue(2147483648.0, "n").int() }.exceptionOrNull()
        check(int?.message == "n must be an Int") { "$int" }
        check(TokamakValue(-2.0.pow(63), "n").long() == Long.MIN_VALUE)
    }

    @Test
    fun cancellation() {
        val replies = mutableListOf<TokamakPluginResult>()
        val helpers = Helpers(Host()) { synchronized(replies) { replies += it } }

        val streamEnded = CountDownLatch(1)
        helpers.stream(flow<Int> { try { awaitCancellation() } finally { streamEnded.countDown() } })()
        check(streamEnded.await(2, TimeUnit.SECONDS)) { "the stream did not end" }

        val callCancelled = CountDownLatch(1)
        helpers.call { try { awaitCancellation() } finally { callCancelled.countDown() } }()
        check(callCancelled.await(2, TimeUnit.SECONDS)) { "the call was not cancelled" }

        // A method that returns after cancellation leaves no reference behind.
        val returned = CountDownLatch(1)
        helpers.resource { withContext(NonCancellable) { delay(20) }; ByteArray(1).also { returned.countDown() } }()
        check(returned.await(2, TimeUnit.SECONDS))
        Thread.sleep(20)
        check(synchronized(replies) { replies.isEmpty() }) { "replies after cancellation: $replies" }
    }

    @Test
    fun listeners() = runBlocking {
        val held = TokamakListeners<Int>(holding = true)
        held.send(1)
        held.send(2)
        val first = mutableListOf<Int>()
        val second = mutableListOf<Int>()
        val a = launch(start = CoroutineStart.UNDISPATCHED) { held.stream().collect { first += it } }
        val b = launch(start = CoroutineStart.UNDISPATCHED) { held.stream().collect { second += it } }
        held.send(3)
        yield()
        check(first == listOf(1, 2, 3) && second == listOf(3)) { "$first $second" }
        a.cancelAndJoin()
        b.cancelAndJoin()
        // Without listeners, values wait again.
        held.send(4)
        check(held.stream().first() == 4)

        // Without listeners, the newest 64 values wait.
        val many = TokamakListeners<Int>(holding = true)
        repeat(70) { many.send(it) }
        check(many.stream().take(64).toList() == (6 until 70).toList())

        val dropping = TokamakListeners<Int>()
        dropping.send(1)
        check(withTimeoutOrNull(50) { dropping.stream().take(1).toList() } == null)
    }

    /** The plugin's code converts the types its methods use to and from JSON, as the Worker sends them. */
    @Test
    fun values() {
        val host = Host()
        val stored = host.json(Zone.Circle(2.0))
        check(stored is JSONObject && stored.similar(JSONObject("""{"kind":"circle","radius":2}"""))) { "$stored" }
        check(host.fromJson(stored, Zone::class) == Zone.Circle(2.0))
        check(host.json(Alert.Volume(1.5)) == 1.5 && host.fromJson("muted", Alert::class) == Alert.Muted)
        val region = host.fromJson(JSONObject("""{"id":"home","radius":5,"tags":[],"counts":{}}"""), Region::class)
        check(region.id == "home" && region.radius == 5.0) { "$region" }
        val unlisted = runCatching { host.json("text") }.exceptionOrNull()
        check(unlisted?.message == "android/Exports.kt uses no String") { "$unlisted" }
        val wrong = runCatching { host.fromJson("loud", Alert::class) }.exceptionOrNull()
        check(wrong?.message == "json must be one of Alert's forms") { "$wrong" }
    }

    /** The plugin's events reach the Worker as its registration lists them, and replies decode as the emitter asks. */
    @Test
    fun events() {
        val host = Host()
        val dispatch = TokamakExamplePluginDispatch
        val region = Region("home", 50.0, null, emptyList(), emptyMap(), null)
        val entered = dispatch.eventPayload("entered", region).toString(Charsets.UTF_8)
        check(entered.contains("\"id\":\"home\"") && !entered.contains("onEntry")) { entered }
        val mistakes = listOf(
            Triple("entered", "home", "event entered takes Region, not String"),
            Triple("left", region, "android/Exports.kt lists no event left"),
        )
        for ((name, event, message) in mistakes) {
            val error = runCatching { dispatch.eventPayload(name, event) }.exceptionOrNull() as TokamakPluginError
            check(error.errorName == "TypeError" && error.message == message) { error.message.orEmpty() }
        }

        /** The listeners' [reply] to the event [name] as a [type], or the failure's message. */
        fun <R : Any> answer(reply: String, type: KClass<R>, name: String = "asked"): String {
            host.reply = reply.toByteArray()
            return runCatching { host.emit(name, region, type, 1_000).toString() }.getOrElse { it.message.orEmpty() }
        }
        check(answer("\"precise\"", Precision::class) == "precise" && host.emitted.last() == "asked")
        check(answer("", Precision::class) == "null" && answer("null", Precision::class) == "null")
        check(answer("\"vague\"", Precision::class).startsWith("reply "))
        answer("\"precise\"", String::class).let { check(it == "event asked replies with Precision, not String") { it } }
        answer("\"precise\"", Precision::class, "entered").let { check(it == "android/Exports.kt lists no reply to event entered") { it } }
        // An event without a reply ignores what its listeners return.
        host.emit("entered", region, 1_000)
    }

    /** A plugin whose dispatch helpers the tests call directly, on the calling thread. */
    private class Plain : TokamakPlugin {
        override val id = "plain"

        override fun executor(method: String) = Executor { it.run() }
    }

    private class Helpers(host: TokamakHost, private val reply: TokamakPluginBytesReply) :
        TokamakDispatch<Plain>(Plain(), host) {
        fun stream(values: Flow<Int>) = collect("stream", reply, values)

        fun call(block: suspend () -> Any?) = launch("call", reply, block)

        fun resource(block: suspend () -> Any) = retain("resource", reply, block)
    }

    private class Host : TokamakHost {
        override val context: Context get() = error("unused")
        override val activity: Activity? = null
        override val isInForeground = true
        override val resources = TokamakResources()

        override fun requireForegroundActivity(): Activity = error("unused")

        override fun wouldPrompt(permission: String) = false

        override fun requestPermissions(permissions: Set<String>, callback: (Map<String, Boolean>) -> Unit) = error("unused")

        /** The events emitted, by name, which the Worker's listeners answer with [reply]. */
        val emitted = mutableListOf<String>()
        var reply = ByteArray(0)

        override fun emitBytes(name: String, payload: ByteArray, timeoutMillis: Long): ByteArray {
            emitted += name
            return reply
        }

        override fun emit(name: String, event: Any, timeoutMillis: Long) {
            emitBytes(name, TokamakExamplePluginDispatch.eventPayload(name, event), timeoutMillis)
        }

        override fun <R : Any> emit(name: String, event: Any, reply: KClass<R>, timeoutMillis: Long): R? {
            val payload = emitBytes(name, TokamakExamplePluginDispatch.eventPayload(name, event), timeoutMillis)
            return TokamakExamplePluginDispatch.reply(name, payload, reply)
        }

        override fun json(value: Any): Any = TokamakExamplePluginDispatch.encodeValue(value)

        override fun <T : Any> fromJson(json: Any?, type: KClass<T>): T = TokamakExamplePluginDispatch.fromJson(json, type)

        override fun fetch(path: String, timeoutMillis: Long): ByteArray = error("unused")
    }
}
