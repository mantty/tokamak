package tokamak.plugins

import com.tokamak.runtime.TokamakHost
import com.tokamak.runtime.TokamakPlugin
import com.tokamak.runtime.TokamakPluginReply

class {{class}}(host: TokamakHost) : TokamakPlugin {
    override val id = "{{id}}"

    override fun call(method: String, arguments: Any?, reply: TokamakPluginReply): (() -> Unit)? {
        if (method != "echo") return super.call(method, arguments, reply)
        reply(Result.success(arguments))
        return null
    }
}
