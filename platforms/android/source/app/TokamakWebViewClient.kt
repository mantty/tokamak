package com.tokamak.runtime

import android.app.Activity
import android.content.Intent
import android.graphics.Bitmap
import android.net.Uri
import android.webkit.ClientCertRequest
import android.webkit.WebResourceRequest
import android.webkit.WebView
import android.webkit.WebViewClient
import java.io.ByteArrayInputStream
import java.security.KeyFactory
import java.security.PrivateKey
import java.security.cert.CertificateFactory
import java.security.cert.X509Certificate
import java.security.spec.PKCS8EncodedKeySpec

/**
 * Answers the app origin's client certificate requests with the runtime's decision. The network
 * security config trusts the gateway's certificate, so no certificate error reaches the app.
 */
internal class TokamakWebViewClient(
    private val activity: Activity,
    private val host: String,
    private val runtime: TokamakRuntime,
) : WebViewClient() {
    override fun onPageStarted(view: WebView, url: String, favicon: Bitmap?) {
        val uri = Uri.parse(url)
        if (!uri.isAppOrigin(host)) {
            view.stopLoading()
            openExternal(uri)
        }
    }

    override fun shouldOverrideUrlLoading(
        view: WebView,
        request: WebResourceRequest,
    ): Boolean {
        if (!request.isForMainFrame || request.url.isAppOrigin(host)) return false
        openExternal(request.url)
        return true
    }

    override fun onReceivedClientCertRequest(view: WebView, request: ClientCertRequest) {
        // Android reports no failure count, so every request is a first attempt.
        val identity = runtime.clientIdentity(request.host, 0)
        if (identity == null) {
            // The gateway requires a certificate, so proceeding without one
            // ends the connection, which is the intended outcome.
            request.ignore()
            return
        }
        val certificate = certificate(identity[0])
        val privateKey = privateKey(identity[1])
        if (certificate == null || privateKey == null) {
            request.cancel()
            return
        }
        request.proceed(privateKey, arrayOf(certificate))
    }

    private fun openExternal(uri: Uri) {
        runCatching {
            activity.startActivity(Intent(Intent.ACTION_VIEW, uri))
        }.onFailure { error ->
            android.util.Log.w("tokamak", "could not open external URL", error)
        }
    }

    private fun certificate(der: ByteArray): X509Certificate? =
        runCatching {
            CertificateFactory.getInstance("X.509")
                .generateCertificate(ByteArrayInputStream(der)) as X509Certificate
        }.getOrNull()

    private fun privateKey(der: ByteArray): PrivateKey? =
        runCatching {
            KeyFactory.getInstance("EC").generatePrivate(PKCS8EncodedKeySpec(der))
        }.getOrNull()
}
