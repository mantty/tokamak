package {{package}}

import com.tokamak.runtime.TokamakHost
import com.tokamak.runtime.TokamakPlugin
import kotlin.concurrent.thread

class {{class}}(private val host: TokamakHost) : TokamakPlugin {
    /** A message the native implementation echoed. */
    data class Echo(val message: String)

    override val id = "{{id}}"

    /** Returns [message], which the Worker's `onEchoed` listeners also receive. */
    fun echo(message: String): String {
        // emit blocks until the Worker's listeners return.
        thread { runCatching { host.emit("echoed", Echo(message), 10_000) } }
        return message
    }
}
