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
    val requests = TokamakPluginRequests(mapOf("test" to plugin), pull = {}) { _, result ->
        check(Thread.currentThread().name != "main")
        check(result.payload.contentEquals(byteArrayOf(0, -1)))
        replied.countDown()
    }
    requests.run(1, TokamakPluginRequest.CALL, "test", "echo", byteArrayOf(0, -1))
    check(replied.await(2, TimeUnit.SECONDS))
    requests.run(2, TokamakPluginRequest.SUBSCRIBE, "test", "once", byteArrayOf(0, -1))
    check(cancelled.await(2, TimeUnit.SECONDS))
    requests.cancel(1)
    requests.cancel(2)
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
        val calls = TokamakPluginRequests(mapOf("gated" to gated), pull = {}) { _, _ -> error("Cancelled request replied") }
        calls.run(1, if (subscribe) TokamakPluginRequest.SUBSCRIBE else TokamakPluginRequest.CALL, "gated", "wait", byteArrayOf())
        check(entered.await(2, TimeUnit.SECONDS))
        calls.cancel(1)
        calls.cancel(1)
        release.countDown()
        check(cleaned.await(2, TimeUnit.SECONDS))
        serial.shutdown()
        check(serial.awaitTermination(2, TimeUnit.SECONDS))
        check(cleanups == 1)
    }
    testResources()
    testBodies()
    println("Passed off-main calls, replies, terminal streams, cancellation before hook installation, resources and bodies")
}

/** A retained object is shared as itself until its release. */
fun testResources() {
    val resources = TokamakResources()
    val frame = byteArrayOf(0, -1, 7)
    var id: Any? = null
    val release = resources.retain(frame) { id = decodePluginValue(it.payload) }
    check(resources.get(id, ByteArray::class.java) === frame)
    check(failure { resources.get(id, String::class.java) } == "TypeError")
    release()
    check(failure { resources.get(id, ByteArray::class.java) } == "InvalidStateError")
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
    val requests = TokamakPluginRequests(mapOf("reader" to reader), pull = { pulls.put(it) }) { _, result -> replies.put(result) }
    requests.run(1, TokamakPluginRequest.CALL_WITH_BODY, "reader", "sum", byteArrayOf())
    for (chunk in listOf(TokamakPluginResult(byteArrayOf(1, 2), false, false), TokamakPluginResult(byteArrayOf(3), false, false), TokamakPluginResult(byteArrayOf(), false, true))) {
        check(pulls.poll(2, TimeUnit.SECONDS) == 1L)
        requests.chunk(1, chunk)
    }
    check(replies.poll(2, TimeUnit.SECONDS)?.payload?.contentEquals(byteArrayOf(6)) == true)
    executor.shutdown()
}
