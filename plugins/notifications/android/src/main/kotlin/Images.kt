package com.tokamak.plugins.notifications

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.os.SystemClock
import android.util.Log
import com.tokamak.runtime.TokamakHost
import java.io.ByteArrayOutputStream
import java.io.IOException
import java.net.URL
import java.util.concurrent.ExecutionException
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import javax.net.ssl.HttpsURLConnection

/** The largest image file the plugin loads. */
private const val MAX_BYTES = 10 * 1024 * 1024

/** The longest side of a decoded image; Android scales notification images down further. */
private const val MAX_SIDE = 1024

/** Loads the images notifications show, from `https` URLs or paths the app serves. */
internal class Images(private val host: TokamakHost) {
    private val threads = Executors.newCachedThreadPool()

    /**
     * Loads [sources] in parallel, then calls [done] on the main thread with those that loaded by
     * [deadline], in [SystemClock.elapsedRealtime] milliseconds.
     */
    fun load(sources: Collection<String>, deadline: Long, done: (Map<String, Bitmap>) -> Unit) {
        val loads = sources.associateWith { source -> threads.submit<Bitmap> { decode(bytes(source, deadline)) } }
        threads.execute {
            val loaded = loads.mapNotNull { (source, load) ->
                runCatching { source to load.get(remaining(deadline), TimeUnit.MILLISECONDS) }
                    .onFailure { Log.w("tokamak", "notification image $source failed: ${(it as? ExecutionException)?.cause ?: it}") }
                    .getOrNull()
            }
            host.context.mainExecutor.execute { done(loaded.toMap()) }
        }
    }

    private fun bytes(source: String, deadline: Long): ByteArray =
        if (source.startsWith("/")) host.fetch(source, remaining(deadline)) else download(source, deadline)

    private fun download(source: String, deadline: Long): ByteArray {
        val connection = URL(source).openConnection() as? HttpsURLConnection ?: throw IOException("the image URL is not https")
        connection.connectTimeout = remaining(deadline).toInt()
        connection.readTimeout = remaining(deadline).toInt()
        try {
            if (connection.responseCode !in 200..299) throw IOException("the server answered ${connection.responseCode}")
            val body = ByteArrayOutputStream()
            val buffer = ByteArray(64 * 1024)
            connection.inputStream.use { input ->
                while (true) {
                    val count = input.read(buffer)
                    if (count < 0) break
                    body.write(buffer, 0, count)
                    if (body.size() > MAX_BYTES) throw IOException("the image is larger than 10 MB")
                    // The read timeout applies to each read, not to the download.
                    if (SystemClock.elapsedRealtime() > deadline) throw IOException("the image did not load in time")
                }
            }
            return body.toByteArray()
        } finally {
            connection.disconnect()
        }
    }

    private fun decode(bytes: ByteArray): Bitmap {
        val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        BitmapFactory.decodeByteArray(bytes, 0, bytes.size, bounds)
        var sample = 1
        while (maxOf(bounds.outWidth, bounds.outHeight) / sample > MAX_SIDE) sample *= 2
        val options = BitmapFactory.Options().apply { inSampleSize = sample }
        return BitmapFactory.decodeByteArray(bytes, 0, bytes.size, options) ?: throw IOException("the file is not an image")
    }

    /** The milliseconds until [deadline], at least 1: a connection's timeout of 0 never expires. */
    private fun remaining(deadline: Long): Long = (deadline - SystemClock.elapsedRealtime()).coerceAtLeast(1)
}
