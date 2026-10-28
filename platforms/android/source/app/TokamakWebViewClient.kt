package com.tokamak.runtime

import android.app.Activity
import android.content.Intent
import android.graphics.Bitmap
import android.net.Uri
import android.net.http.SslError
import android.webkit.ClientCertRequest
import android.webkit.SslErrorHandler
import android.webkit.WebResourceRequest
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

    override fun onReceivedSslError(view: WebView, handler: SslErrorHandler, error: SslError) {
        val host = Uri.parse(error.url).host
        val authority = host?.let(runtime::serverAuthority)
        val trusted = authority != null && chainsTo(error, authority)
        if (trusted) {
            android.util.Log.i("tokamak", "trusted the gateway's certificate for $host (${error.primaryError})")
            handler.proceed()
            return
        }
        if (authority != null) android.util.Log.w("tokamak", "refused the gateway's certificate: $error")
        handler.cancel()
    }

    override fun onReceivedClientCertRequest(view: WebView, request: ClientCertRequest) {
        val identity = runtime.clientIdentity(request.host)
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

    /**
     * Whether the runtime's authority signed `error`'s certificate, allowing only the failures
     * reported for an untrusted certificate or a wrong device clock: the runtime issues and renews
     * the certificate itself, so its signature is the trust decision. The primary error is the most
     * severe one, which hides a lesser hostname mismatch, so that is checked on its own.
     */
    private fun chainsTo(error: SslError, authority: ByteArray): Boolean {
        if (!certificateError(error.primaryError)) return false
        if (error.hasError(SslError.SSL_IDMISMATCH)) return false
        val presented = error.certificate.x509Certificate ?: return false
        val issuer = certificate(authority) ?: return false
        return runCatching { presented.verify(issuer.publicKey) }.isSuccess
    }

    /** Whether `primary` names the certificate as untrusted or wrongly dated. */
    private fun certificateError(primary: Int): Boolean =
        primary == SslError.SSL_UNTRUSTED ||
            primary == SslError.SSL_NOTYETVALID ||
            primary == SslError.SSL_EXPIRED ||
            primary == SslError.SSL_DATE_INVALID
}
