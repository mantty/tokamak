package com.tokamak.runtime

import android.Manifest
import android.app.Activity
import android.content.pm.PackageManager
import android.webkit.PermissionRequest
import android.webkit.WebChromeClient

private const val MEDIA_PERMISSION_REQUEST = 0x3ED1

private val RESOURCE_PERMISSIONS =
    mapOf(
        PermissionRequest.RESOURCE_VIDEO_CAPTURE to Manifest.permission.CAMERA,
        PermissionRequest.RESOURCE_AUDIO_CAPTURE to Manifest.permission.RECORD_AUDIO,
    )

/** Grants the app origin's media capture requests once the app holds their permissions. */
internal class TokamakWebChromeClient(
    private val activity: Activity,
    private val host: String,
) : WebChromeClient() {
    private val waiting = mutableListOf<PermissionRequest>()

    /** The permissions the open permission dialog asks for; empty while none is open. */
    private var asking = emptySet<String>()

    override fun onPermissionRequest(request: PermissionRequest) {
        val supported = request.resources.all(RESOURCE_PERMISSIONS::containsKey)
        if (!request.origin.isAppOrigin(host) || !supported) {
            request.deny()
            return
        }
        if (missingPermissions(request).isEmpty()) {
            request.grant(request.resources)
            return
        }
        waiting += request
        if (asking.isEmpty()) askForMissingPermissions()
    }

    override fun onPermissionRequestCanceled(request: PermissionRequest) {
        waiting -= request
    }

    fun onRequestPermissionsResult(requestCode: Int) {
        if (requestCode != MEDIA_PERMISSION_REQUEST) return
        val answered = waiting.filter { asking.containsAll(missingPermissions(it)) }
        waiting -= answered
        answered.forEach(::decide)
        askForMissingPermissions()
    }

    private fun askForMissingPermissions() {
        asking = waiting.flatMap(::missingPermissions).toSet()
        if (asking.isNotEmpty()) {
            activity.requestPermissions(asking.toTypedArray(), MEDIA_PERMISSION_REQUEST)
        }
    }

    private fun decide(request: PermissionRequest) {
        if (missingPermissions(request).isEmpty()) request.grant(request.resources) else request.deny()
    }

    private fun missingPermissions(request: PermissionRequest): List<String> =
        request.resources.map(RESOURCE_PERMISSIONS::getValue).filterNot(::isGranted)

    private fun isGranted(permission: String): Boolean =
        activity.checkSelfPermission(permission) == PackageManager.PERMISSION_GRANTED
}
