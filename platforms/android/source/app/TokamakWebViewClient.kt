package com.tokamak.runtime

import android.app.Activity
import android.content.Intent
import android.graphics.Bitmap
import android.net.Uri
import android.net.http.SslError
import android.webkit.ClientCertRequest
import android.webkit.WebResourceRequest
import android.webkit.SslErrorHandler
import android.webkit.WebView
import android.webkit.WebViewClient
import java.io.ByteArrayInputStream
import java.security.KeyFactory
import java.security.PrivateKey
import java.security.cert.CertificateFactory
import java.security.cert.X509Certificate
import java.security.spec.PKCS8EncodedKeySpec

/** Answers the app origin's TLS challenges with the runtime's decision. */
internal class TokamakWebViewClient(
    private val activity: Activity,
    private val host: String,
    private val runtime: TokamakRuntime,
    private val pluginBridge: TokamakPluginBridge,
) : WebViewClient() {
    override fun onPageStarted(view: WebView, url: String, favicon: Bitmap?) {
        pluginBridge.close()
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

    override fun onReceivedSslError(view: WebView, handler: SslErrorHandler, error: SslError) {
        val host = Uri.parse(error.url).host
        val authority = host?.let(runtime::serverAuthority)
        val trusted = authority != null && chainsTo(error, authority)
        if (trusted) handler.proceed() else handler.cancel()
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

    private fun chainsTo(error: SslError, authority: ByteArray): Boolean {
        if (error.primaryError != SslError.SSL_UNTRUSTED) return false
        val presented = error.certificate.x509Certificate ?: return false
        val issuer = certificate(authority) ?: return false
        return runCatching {
            presented.checkValidity()
            presented.verify(issuer.publicKey)
        }.isSuccess
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
