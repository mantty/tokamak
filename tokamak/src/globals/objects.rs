use super::checked;
use rquickjs::{Ctx, Value, qjs};
use tokamak_macros::host_function;

// The class name comes from the engine, not a user-controlled toStringTag.
#[allow(clippy::needless_pass_by_value)]
#[host_function]
pub(super) fn class_name<'js>(
    ctx: Ctx<'js>,
    value: Value<'js>,
) -> rquickjs::Result<Option<rquickjs::String<'js>>> {
    if value.is_proxy() {
        return rquickjs::String::from_str(ctx, "Proxy").map(Some);
    }
    if !value.is_object() {
        return Ok(None);
    }
    // SAFETY: The value belongs to this context. GetClassName returns an owned
    // atom, released after converting it to a separately owned string value.
    let name = unsafe {
        let raw_ctx = ctx.as_raw().as_ptr();
        let class = qjs::JS_GetClassID(value.as_raw());
        let atom = qjs::JS_GetClassName(qjs::JS_GetRuntime(raw_ctx), class);
        let name = qjs::JS_AtomToString(raw_ctx, atom);
        qjs::JS_FreeAtom(raw_ctx, atom);
        Value::from_raw(ctx, name)
    };
    Ok(checked(name)?.into_string())
}

#[cfg(test)]
mod tests {
    #[test]
    fn reports_engine_class_without_user_getters() -> rquickjs::Result<()> {
        let runtime = rquickjs::Runtime::new()?;
        let context = rquickjs::Context::full(&runtime)?;
        context.with(|ctx| {
            ctx.globals().set("className", rquickjs::Function::new(ctx.clone(), super::class_name)?)?;
            let names: Vec<String> = ctx.eval("[{}, [], new Date(), /a/, new Error(), new Uint8Array(), new ArrayBuffer(0)].map(className)")?;
            assert_eq!(names, ["Object", "Array", "Date", "RegExp", "Error", "Uint8Array", "ArrayBuffer"]);
            Ok(())
        })
    }
}
