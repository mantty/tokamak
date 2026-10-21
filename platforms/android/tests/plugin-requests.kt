package com.tokamak.runtime
import java.util.concurrent.*
import java.util.concurrent.atomic.AtomicInteger

fun main() {
    val executor = Executors.newSingleThreadExecutor()
    val replied = CountDownLatch(1)
    val cancelled = CountDownLatch(1)
    val completedCleanups = AtomicInteger()
    val plugin = object : TokamakPlugin {
        override val id = "test"
        override fun executor(method: String) = executor
        override fun callBytes(method: String, payload: ByteArray, reply: TokamakPluginBytesReply): (() -> Unit)? {
            check(Thread.currentThread().name != "main")
            reply(TokamakPluginResult(payload, false, true))
            return { completedCleanups.incrementAndGet(); Unit }
        }
        override fun subscribeBytes(method: String, payload: ByteArray, reply: TokamakPluginBytesReply): () -> Unit {
            check(Thread.currentThread().name != "main")
            reply(TokamakPluginResult(payload, false, true))
            return { check(Thread.currentThread().name != "main"); cancelled.countDown() }
        }
    }
    val requests = TokamakPluginRequests<Long>(mapOf("test" to plugin)) { _, result ->
        check(Thread.currentThread().name != "main")
        check(result.payload.contentEquals(byteArrayOf(0, -1)))
        replied.countDown()
    }
    requests.run(1, TokamakPluginRequest.CALL, "test", "echo", byteArrayOf(0, -1))
    check(replied.await(2, TimeUnit.SECONDS))
    requests.run(2, TokamakPluginRequest.SUBSCRIBE, "test", "once", byteArrayOf(0, -1))
    check(cancelled.await(2, TimeUnit.SECONDS))
    requests.cancelAll()
    executor.shutdown()
    check(executor.awaitTermination(2, TimeUnit.SECONDS))
    check(completedCleanups.get() == 0)
    for (subscribe in listOf(false, true)) {
        val serial = Executors.newSingleThreadExecutor()
        val entered = CountDownLatch(1)
        val release = CountDownLatch(1)
        val cleaned = CountDownLatch(1)
        var cleanups = 0
        val gated = object : TokamakPlugin {
            override val id = "gated"
            override fun executor(method: String) = serial
            fun begin(reply: TokamakPluginBytesReply): () -> Unit {
                entered.countDown()
                check(release.await(2, TimeUnit.SECONDS))
                return {
                    check(Thread.currentThread().name != "main")
                    cleanups++
                    reply(TokamakPluginResult(byteArrayOf(), false, true))
                    cleaned.countDown()
                }
            }
            override fun callBytes(method: String, payload: ByteArray, reply: TokamakPluginBytesReply) = begin(reply)
            override fun subscribeBytes(method: String, payload: ByteArray, reply: TokamakPluginBytesReply) = begin(reply)
        }
        val calls = TokamakPluginRequests<Int>(mapOf("gated" to gated)) { _, _ -> error("Cancelled request replied") }
        calls.run(1, if (subscribe) TokamakPluginRequest.SUBSCRIBE else TokamakPluginRequest.CALL, "gated", "wait", byteArrayOf())
        check(entered.await(2, TimeUnit.SECONDS))
        if (subscribe) { calls.cancelAll(); calls.cancelAll() } else { calls.cancel(1); calls.cancel(1) }
        release.countDown()
        check(cleaned.await(2, TimeUnit.SECONDS))
        serial.shutdown()
        check(serial.awaitTermination(2, TimeUnit.SECONDS))
        check(cleanups == 1)
    }
    testResources()
    testBodies()
    testBindings()
    println("Passed off-main calls, replies, terminal streams, cancellation before hook installation, resources, bodies and bindings")
}

/** A retained object is shared as itself until its release. */
fun testResources() {
    val resources = TokamakResources()
    val frame = byteArrayOf(0, -1, 7)
    var id: Any? = null
    val release = resources.retain(frame) { id = it.getOrThrow() }
    check(resources.get(id, ByteArray::class.java) === frame)
    check(failure { resources.get(id, String::class.java) } == "TypeError")
    release()
    check(failure { resources.get(id, ByteArray::class.java) } == "InvalidStateError")
}

/**
 * Bindings keep returned objects in the invocation's scope, convert values both ways, report
 * exceptions by class, and implement interfaces for the handlers that exist.
 */
fun testBindings() {
    val resources = TokamakResources()
    val replies = LinkedBlockingQueue<TokamakPluginResult>()
    fun binding(payload: String) = TokamakBinding(payload.toByteArray(), resources) { replies.put(it) }
    fun reply(): TokamakPluginResult = replies.poll(2, TimeUnit.SECONDS)!!
    fun json(result: TokamakPluginResult) = org.json.JSONTokener(String(result.payload)).nextValue()
    fun plain(value: Any?): Any? = when (value) {
        is org.json.JSONObject -> value.keys().asSequence().associateWith { plain(value.get(it)) }
        is org.json.JSONArray -> (0 until value.length()).map { plain(value.get(it)) }
        else -> value
    }
    val scope = resources.open()

    val created = binding("""{"scope":"$scope","arguments":[]}""")
    created.run { created.encode(java.util.concurrent.atomic.AtomicInteger(1)) }
    val reference = json(reply()) as org.json.JSONObject
    check(reference.getString("#") == "object" && reference.getJSONArray("types").join(",") ==
        "\"java.util.concurrent.atomic.AtomicInteger\",\"java.lang.Number\",\"java.lang.Object\",\"java.io.Serializable\"")
    val target = """{"#":"object","id":"${reference.getString("id")}"}"""

    val appended = binding("""{"target":$target,"arguments":[3,"cd",{"#":"bytes","value":"AQI="},null,{"#":"number","value":"Infinity"}]}""")
    appended.run {
        check(appended.int(appended[0]) == 3 && appended.bytes(appended[2]).contentEquals(byteArrayOf(1, 2)))
        check(appended.optional(appended[3]) { appended.int(it) } == null && appended.double(appended[4]).isInfinite())
        check(appended.string(appended[1]) == "cd")
        appended.target(java.util.concurrent.atomic.AtomicInteger::class.java).addAndGet(appended.int(appended[0]))
    }
    check(json(reply()) == 4)

    val values = binding("{}")
    values.run { values.encode(listOf(Double.NaN, mapOf("#" to 1), byteArrayOf(1, 2))) }
    check(plain(json(reply())) == listOf(mapOf("#" to "number", "value" to "NaN"), mapOf("#" to "record", "value" to mapOf("#" to 1)), mapOf("#" to "bytes", "value" to "AQI=")))

    val failing = binding("{}")
    failing.run { throw SecurityException("Denied") }
    val failure = reply()
    check(failure.failed && String(failure.payload).contains("java.lang.SecurityException"))

    val observed = binding("""{"scope":"$scope","methods":["run"]}""")
    observed.implement { binding -> Runnable { binding.call("run") { listOf() } } }
    val runnable = resources.get(json(reply()), Runnable::class.java)
    runnable.run()
    check(plain(json(reply())) == mapOf("method" to "run", "arguments" to listOf<Any?>()))
    val ignored = binding("""{"scope":"$scope","methods":[]}""")
    check(!ignored.call("run") { listOf() } && replies.isEmpty())

    resources.release(reference.getString("id"))
    check(failure { resources.get(reference.getString("id"), Any::class.java) } == "InvalidStateError")
    val kept = (created.encode(Any()) as org.json.JSONObject).getString("id")
    resources.end(scope)
    check(failure { resources.get(kept, Any::class.java) } == "InvalidStateError")
}

fun failure(operation: () -> Unit): String? =
    runCatching(operation).exceptionOrNull()?.let { (it as TokamakPluginError).errorName }

/** A call reads its body one chunk per pull, on its executor, until the last chunk. */
fun testBodies() {
    val executor = Executors.newSingleThreadExecutor()
    val pulls = LinkedBlockingQueue<Long>()
    val replies = LinkedBlockingQueue<TokamakPluginResult>()
    val reader = object : TokamakPlugin {
        override val id = "reader"
        override fun executor(method: String) = executor
        override fun callBytes(method: String, payload: ByteArray, body: TokamakPluginBody, reply: TokamakPluginBytesReply): (() -> Unit)? {
            var sum = 0
            fun next() {
                body.read { chunk ->
                    check(Thread.currentThread().name != "main")
                    val bytes = chunk.getOrThrow()
                    if (bytes == null) reply(TokamakPluginResult(byteArrayOf(sum.toByte()), false, true))
                    else { sum += bytes.sum(); next() }
                }
            }
            next()
            return null
        }
    }
    val requests = TokamakPluginRequests<Long>(mapOf("reader" to reader), pull = { pulls.put(it) }) { _, result -> replies.put(result) }
    requests.run(1, TokamakPluginRequest.CALL_WITH_BODY, "reader", "sum", byteArrayOf())
    for (chunk in listOf(TokamakPluginResult(byteArrayOf(1, 2), false, false), TokamakPluginResult(byteArrayOf(3), false, false), TokamakPluginResult(byteArrayOf(), false, true))) {
        check(pulls.poll(2, TimeUnit.SECONDS) == 1L)
        requests.chunk(1, chunk)
    }
    check(replies.poll(2, TimeUnit.SECONDS)?.payload?.contentEquals(byteArrayOf(6)) == true)
    executor.shutdown()
}
