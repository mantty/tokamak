package {{package}}

import com.tokamak.runtime.TokamakExporter
import com.tokamak.runtime.TokamakExports

object {{class}}Exports : TokamakExports<{{class}}> {
    override fun exports(e: TokamakExporter<{{class}}>) {
        e.call({{class}}::echo, doc = "Returns a message through the native implementation.")
        e.event("echoed", {{class}}.Echo::class, doc = "Receives each message the native implementation echoed.")
    }
}
