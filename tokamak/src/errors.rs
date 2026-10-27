//! `Error` objects created from Rust.

use rquickjs::object::Property;
use rquickjs::{Ctx, Exception};

/// A new `Error` whose `message`, as `new Error(message)` gives it, is not
/// enumerable.
#[expect(
    clippy::disallowed_methods,
    reason = "the one use, which makes `message` not enumerable"
)]
pub(crate) fn new_error<'js>(ctx: &Ctx<'js>, message: &str) -> rquickjs::Result<Exception<'js>> {
    // `from_message` sets `message` as an enumerable property.
    let exception = Exception::from_message(ctx.clone(), message)?;
    let object = exception.as_object();
    object.remove("message")?;
    object.prop("message", Property::from(message).writable().configurable())?;
    Ok(exception)
}

/// Throws `new_error(message)`.
pub(crate) fn throw_message(ctx: &Ctx<'_>, message: &str) -> rquickjs::Error {
    match new_error(ctx, message) {
        Ok(exception) => exception.throw(),
        Err(error) => error,
    }
}

#[cfg(test)]
mod tests {
    use rquickjs::{Context, Object, Runtime};

    #[test]
    fn creates_errors_as_javascript_does() -> Result<(), Box<dyn std::error::Error>> {
        let runtime = Runtime::new()?;
        let context = Context::full(&runtime)?;
        context.with(|ctx| -> Result<(), Box<dyn std::error::Error>> {
            let _thrown = super::throw_message(&ctx, "failed");
            ctx.globals().set("thrown", ctx.catch())?;
            let described: Object = ctx.eval(
                r#"({
                  native: Object.getOwnPropertyDescriptor(thrown, "message"),
                  script: Object.getOwnPropertyDescriptor(new Error("failed"), "message"),
                  error: thrown instanceof Error,
                })"#,
            )?;
            let json = ctx.json_stringify(described)?.ok_or("no JSON")?.to_string()?;
            assert_eq!(
                json,
                r#"{"native":{"value":"failed","writable":true,"enumerable":false,"configurable":true},"script":{"value":"failed","writable":true,"enumerable":false,"configurable":true},"error":true}"#
            );
            Ok(())
        })
    }
}
