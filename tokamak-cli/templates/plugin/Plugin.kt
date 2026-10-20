package tokamak.plugins

import com.tokamak.runtime.TokamakHost
import com.tokamak.runtime.TokamakPlugin
import com.tokamak.runtime.TokamakPluginReply
import java.util.concurrent.Executors

class {{class}}(host: TokamakHost) : TokamakPlugin {
    override val id = "{{id}}"
    private val executor = Executors.newSingleThreadExecutor()
    override fun executor(method: String) = executor

    override fun call(method: String, arguments: Any?, reply: TokamakPluginReply): (() -> Unit)? {
        if (method != "echo") return super.call(method, arguments, reply)
        reply(Result.success(arguments))
        return null
    }
}
