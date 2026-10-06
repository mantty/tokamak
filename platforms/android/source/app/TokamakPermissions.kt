package com.tokamak.runtime

import android.content.pm.PackageManager

/**
 * Asks for runtime permissions in the app's activity one request at a time, as Android cancels a
 * request made while another shows. Use it on the main thread.
 */
internal class TokamakPermissions(private val host: TokamakHost) {
    private class Request(val permissions: Set<String>, val callback: (Map<String, Boolean>) -> Unit)

    private val queue = ArrayDeque<Request>()

    /** Identifies the request showing, the head of [queue]. */
    private var requestCode = 0

    fun request(permissions: Set<String>, callback: (Map<String, Boolean>) -> Unit) {
        queue.addLast(Request(permissions, callback))
        if (queue.size == 1) askNext()
    }

    fun onRequestPermissionsResult(requestCode: Int) {
        if (requestCode != this.requestCode || queue.isEmpty()) return
        val answered = queue.removeFirst()
        askNext()
        answer(answered)
    }

    /** Ends every request, answering with the permissions' current state. */
    fun cancel() {
        requestCode += 1
        val cancelled = queue.toList()
        queue.clear()
        cancelled.forEach(::answer)
    }

    private fun askNext() {
        val request = queue.firstOrNull() ?: return
        val activity = host.activity ?: return cancel()
        requestCode += 1
        activity.requestPermissions(request.permissions.toTypedArray(), requestCode)
    }

    private fun answer(request: Request) =
        request.callback(request.permissions.associateWith(::isGranted))

    private fun isGranted(permission: String): Boolean =
        host.context.checkSelfPermission(permission) == PackageManager.PERMISSION_GRANTED
}
