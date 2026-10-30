package com.tokamak.plugins.example

import com.tokamak.runtime.TokamakExporter
import com.tokamak.runtime.TokamakExports
import com.tokamak.runtime.TokamakOverflow

object ExampleExports : TokamakExports<TokamakExamplePlugin> {
    override fun exports(e: TokamakExporter<TokamakExamplePlugin>) {
        e.call(TokamakExamplePlugin::addRegion, doc = "Monitors a circular region until `removeRegion`.")
        e.call(TokamakExamplePlugin::regions)
        e.call(TokamakExamplePlugin::permission, doc = "The app's permission.\n\nNever asks.")
        e.stream(TokamakExamplePlugin::watchPosition, overflow = TokamakOverflow.DROP_OLDEST)
        e.stream(TokamakExamplePlugin::onVisit)
        e.resource(TokamakExamplePlugin::capture)
        e.call(TokamakExamplePlugin::encode)
        e.call(TokamakExamplePlugin::upload)
        e.call(TokamakExamplePlugin::digest)
        e.call(TokamakExamplePlugin::read)
        e.call(TokamakExamplePlugin::copy)
        e.call(TokamakExamplePlugin::mirror)
        e.call(TokamakExamplePlugin::alert)
        e.call(TokamakExamplePlugin::zone)
        e.call(TokamakExamplePlugin::mark)
        e.event("entered", Region::class, doc = "Each region the device enters.")
        e.event("asked", Region::class, reply = Precision::class)
        e.event("chose", Precision::class)
        e.event("visited", Visit::class)
    }
}
