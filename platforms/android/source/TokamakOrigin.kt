package com.tokamak.runtime

import android.net.Uri

/** Whether this URI is on the app origin, `https://<host>`. */
internal fun Uri.isAppOrigin(host: String): Boolean =
    scheme.equals("https", ignoreCase = true) &&
        this.host.equals(host, ignoreCase = true) &&
        (port == -1 || port == 443)
