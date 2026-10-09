package com.tokamak.runtime

import android.content.Context
import android.content.pm.PackageManager

private const val PREFERENCES = "tokamak.permissions"
private const val REFUSED = "refused"

/**
 * Asks for runtime permissions in the app's activity one request at a time, as Android cancels a
 * request made while another shows. Remembers the permissions the user has refused for good, as
 * Android reports them the same way as permissions never asked for. Use it on the main thread.
 */
internal class TokamakPermissions(private val app: TokamakApplication) {
    private class Request(val permissions: Set<String>, val callback: (Map<String, Boolean>) -> Unit)

    private val queue = ArrayDeque<Request>()
    private val preferences by lazy { app.getSharedPreferences(PREFERENCES, Context.MODE_PRIVATE) }

    /** Identifies the request showing, the head of [queue]. */
    private var requestCode = 0

    /**
     * Asks for [permissions], then passes [callback] whether each is granted. Answers at once when
     * none would prompt; throws `NeedsUIError` when one would while the app is in the background.
     */
    fun request(permissions: Set<String>, callback: (Map<String, Boolean>) -> Unit) {
        val request = Request(permissions, callback)
        if (permissions.none(::wouldPrompt)) return answer(request)
        app.requireForegroundActivity()
        queue.addLast(request)
        if (queue.size == 1) askNext()
    }

    /** Whether asking for [permission] shows the system prompt. */
    fun wouldPrompt(permission: String): Boolean =
        !isGranted(permission) &&
            (app.activity?.shouldShowRequestPermissionRationale(permission) == true || permission !in refused())

    fun onRequestPermissionsResult(requestCode: Int) {
        if (requestCode != this.requestCode || queue.isEmpty()) return
        val answered = queue.removeFirst()
        remember(answered.permissions)
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
        val activity = app.activity ?: return cancel()
        requestCode += 1
        activity.requestPermissions(request.permissions.toTypedArray(), requestCode)
    }

    /** Records which of the answered [permissions] the user refused for good. */
    private fun remember(permissions: Set<String>) {
        val activity = app.activity ?: return
        val refusedForGood =
            permissions.filter { !isGranted(it) && !activity.shouldShowRequestPermissionRationale(it) }
        preferences.edit().putStringSet(REFUSED, refused() - permissions + refusedForGood).apply()
    }

    private fun answer(request: Request) =
        request.callback(request.permissions.associateWith(::isGranted))

    private fun refused(): Set<String> = preferences.getStringSet(REFUSED, null).orEmpty()

    private fun isGranted(permission: String): Boolean =
        app.checkSelfPermission(permission) == PackageManager.PERMISSION_GRANTED
}
