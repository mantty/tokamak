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

    override fun onPermissionRequest(request: PermissionRequest) {
        val permissions = request.resources.map(RESOURCE_PERMISSIONS::get)
        if (!request.origin.isAppOrigin(host) || null in permissions) {
            request.deny()
            return
        }
        val missing = permissions.filterNotNull().filterNot(::isGranted)
        if (missing.isEmpty()) {
            request.grant(request.resources)
            return
        }
        waiting += request
        if (waiting.size == 1) {
            activity.requestPermissions(missing.toTypedArray(), MEDIA_PERMISSION_REQUEST)
        }
    }

    override fun onPermissionRequestCanceled(request: PermissionRequest) {
        waiting -= request
    }

    fun onRequestPermissionsResult(requestCode: Int) {
        if (requestCode != MEDIA_PERMISSION_REQUEST) return
        val requests = waiting.toList()
        waiting.clear()
        requests.forEach(::decide)
    }

    private fun decide(request: PermissionRequest) {
        val granted = request.resources.all { isGranted(RESOURCE_PERMISSIONS.getValue(it)) }
        if (granted) request.grant(request.resources) else request.deny()
    }

    private fun isGranted(permission: String): Boolean =
        activity.checkSelfPermission(permission) == PackageManager.PERMISSION_GRANTED
}
