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
    requests.run(1, false, "test", "echo", byteArrayOf(0, -1))
    check(replied.await(2, TimeUnit.SECONDS))
    requests.run(2, true, "test", "once", byteArrayOf(0, -1))
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
        calls.run(1, subscribe, "gated", "wait", byteArrayOf())
        check(entered.await(2, TimeUnit.SECONDS))
        if (subscribe) { calls.cancelAll(); calls.cancelAll() } else { calls.cancel(1); calls.cancel(1) }
        release.countDown()
        check(cleaned.await(2, TimeUnit.SECONDS))
        serial.shutdown()
        check(serial.awaitTermination(2, TimeUnit.SECONDS))
        check(cleanups == 1)
    }
    println("Passed off-main calls, replies, terminal streams, and cancellation before hook installation")
}
