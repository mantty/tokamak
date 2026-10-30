package com.tokamak.plugins.localauthentication

import com.tokamak.runtime.TokamakExporter
import com.tokamak.runtime.TokamakExports

object LocalAuthenticationExports : TokamakExports<TokamakLocalAuthenticationPlugin> {
    override fun exports(e: TokamakExporter<TokamakLocalAuthenticationPlugin>) {
        e.call(TokamakLocalAuthenticationPlugin::status, doc = "Whether the device can perform `authentication` now.")
        e.call(
            TokamakLocalAuthenticationPlugin::authenticate,
            doc = "Asks the device owner to authenticate, giving `prompt` as the reason.",
        )
    }
}
