//! Rust work that JavaScript cancels.

use rquickjs::class::Trace;
use rquickjs::{Class, Ctx, JsLifetime};
use tokio_util::sync::CancellationToken;

/// Rust work, such as a timer or a native call, that `cancel` stops if it has
/// not finished.
#[derive(Trace, JsLifetime)]
#[rquickjs::class]
pub(crate) struct Cancellation {
    #[qjs(skip_trace)]
    token: CancellationToken,
}

#[tokamak_macros::js_methods]
#[rquickjs::methods]
impl Cancellation {
    pub fn cancel(&self) {
        self.token.cancel();
    }
}

impl Cancellation {
    /// A handle that cancels `token`.
    pub(crate) fn instance<'js>(
        ctx: &Ctx<'js>,
        token: CancellationToken,
    ) -> rquickjs::Result<Class<'js, Self>> {
        Class::instance(ctx.clone(), Self { token })
    }
}
