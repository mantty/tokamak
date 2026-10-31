//! JavaScript values that rquickjs types only loosely, given TypeScript types
//! by the Rust types that wrap them.

use std::marker::PhantomData;

use rquickjs::class::{Trace, Tracer};
use rquickjs::function::IntoArgs;
use rquickjs::{Ctx, FromJs, Function, IntoJs, Value};

/// A JavaScript function that Rust calls with the tuple `Args` and whose
/// result it reads as `R`.
pub(crate) struct JsFunction<'js, Args, R = ()> {
    function: Function<'js>,
    signature: PhantomData<fn(Args) -> R>,
}

impl<'js, Args: IntoArgs<'js>, R: FromJs<'js>> JsFunction<'js, Args, R> {
    pub(crate) fn call(&self, arguments: Args) -> rquickjs::Result<R> {
        self.function.call(arguments)
    }
}

impl<'js, Args, R> JsFunction<'js, Args, R> {
    pub(crate) fn into_inner(self) -> Function<'js> {
        self.function
    }
}

impl<Args, R> Clone for JsFunction<'_, Args, R> {
    fn clone(&self) -> Self {
        Self {
            function: self.function.clone(),
            signature: PhantomData,
        }
    }
}

impl<'js, Args, R> FromJs<'js> for JsFunction<'js, Args, R> {
    fn from_js(ctx: &Ctx<'js>, value: Value<'js>) -> rquickjs::Result<Self> {
        Ok(Self {
            function: Function::from_js(ctx, value)?,
            signature: PhantomData,
        })
    }
}

impl<'js, Args, R> IntoJs<'js> for JsFunction<'js, Args, R> {
    fn into_js(self, ctx: &Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        self.function.into_js(ctx)
    }
}

impl<'js, Args, R> Trace<'js> for JsFunction<'js, Args, R> {
    fn trace<'a>(&self, tracer: Tracer<'a, 'js>) {
        self.function.trace(tracer);
    }
}

#[cfg(test)]
impl<Args: crate::typescript::JsArguments, R: crate::typescript::JsReturn> crate::typescript::JsType
    for JsFunction<'_, Args, R>
{
    fn typescript(types: &mut crate::typescript::Types) -> String {
        let parameters = Args::parameters(types).join(", ");
        format!("({parameters}) => {}", R::returns(types))
    }
}
