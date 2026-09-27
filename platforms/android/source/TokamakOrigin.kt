package com.tokamak.runtime

import android.net.Uri

/** Whether this URI is on the app origin, `https://<appHost>`. */
internal fun Uri.isAppOrigin(appHost: String): Boolean =
    scheme.equals("https", ignoreCase = true) &&
        host.equals(appHost, ignoreCase = true) &&
        (port == -1 || port == 443)
