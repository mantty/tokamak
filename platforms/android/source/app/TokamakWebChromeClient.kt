package com.tokamak.runtime

import android.Manifest
import android.content.pm.PackageManager
import android.webkit.PermissionRequest
import android.webkit.WebChromeClient

private val RESOURCE_PERMISSIONS =
    mapOf(
        PermissionRequest.RESOURCE_VIDEO_CAPTURE to Manifest.permission.CAMERA,
        PermissionRequest.RESOURCE_AUDIO_CAPTURE to Manifest.permission.RECORD_AUDIO,
    )

/** Grants the app origin's media capture requests once the app holds their permissions. */
internal class TokamakWebChromeClient(
    private val tokamak: TokamakApplication,
    private val host: String,
) : WebChromeClient() {
    private val waiting = mutableSetOf<PermissionRequest>()

    override fun onPermissionRequest(request: PermissionRequest) {
        val supported = request.resources.all(RESOURCE_PERMISSIONS::containsKey)
        if (!request.origin.isAppOrigin(host) || !supported) {
            request.deny()
            return
        }
        val missing = missingPermissions(request)
        if (missing.isEmpty()) {
            request.grant(request.resources)
            return
        }
        waiting += request
        try {
            tokamak.permissionRequests.request(missing.toSet()) { if (waiting.remove(request)) decide(request) }
        } catch (_: TokamakPluginError) {
            waiting -= request
            request.deny()
        }
    }

    override fun onPermissionRequestCanceled(request: PermissionRequest) {
        waiting -= request
    }

    private fun decide(request: PermissionRequest) {
        if (missingPermissions(request).isEmpty()) request.grant(request.resources) else request.deny()
    }

    private fun missingPermissions(request: PermissionRequest): List<String> =
        request.resources.map(RESOURCE_PERMISSIONS::getValue).filterNot(::isGranted)

    private fun isGranted(permission: String): Boolean =
        tokamak.checkSelfPermission(permission) == PackageManager.PERMISSION_GRANTED
}
