//! JavaScript values that rquickjs types only loosely, given TypeScript types
//! by the Rust types that wrap them.

use std::future::Future;
use std::marker::PhantomData;

use rquickjs::class::{Trace, Tracer};
use rquickjs::function::IntoArgs;
use rquickjs::prelude::List;
use rquickjs::{Ctx, FromJs, Function, IntoJs, Object, Promise, Value};

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

/// A JavaScript promise that Rust awaits as, or resolves with, a `T`.
pub(crate) struct JsPromise<'js, T> {
    promise: Promise<'js>,
    result: PhantomData<fn() -> T>,
}

impl<'js, T: IntoJs<'js> + 'js> JsPromise<'js, T> {
    /// The promise of `future`'s result.
    pub(crate) fn wrap_future(
        ctx: &Ctx<'js>,
        future: impl Future<Output = rquickjs::Result<T>> + 'js,
    ) -> rquickjs::Result<Self> {
        Ok(Self {
            promise: Promise::wrap_future(ctx, future)?,
            result: PhantomData,
        })
    }
}

impl<'js, T> JsPromise<'js, T> {
    pub(crate) fn into_inner(self) -> Promise<'js> {
        self.promise
    }
}

impl<'js, T: FromJs<'js>> JsPromise<'js, T> {
    /// The promise's result, once it settles.
    pub(crate) async fn into_future(self) -> rquickjs::Result<T> {
        self.promise.into_future().await
    }
}

/// A promise that code Rust does not declare, such as the Worker's, makes,
/// whose result Rust reads as `T`.
impl<'js, T> From<Promise<'js>> for JsPromise<'js, T> {
    fn from(promise: Promise<'js>) -> Self {
        Self {
            promise,
            result: PhantomData,
        }
    }
}

impl<'js, T> FromJs<'js> for JsPromise<'js, T> {
    fn from_js(ctx: &Ctx<'js>, value: Value<'js>) -> rquickjs::Result<Self> {
        Ok(Self {
            promise: Promise::from_js(ctx, value)?,
            result: PhantomData,
        })
    }
}

impl<'js, T> IntoJs<'js> for JsPromise<'js, T> {
    fn into_js(self, ctx: &Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        self.promise.into_js(ctx)
    }
}

#[cfg(test)]
impl<T: crate::typescript::JsReturn> crate::typescript::JsType for JsPromise<'_, T> {
    fn typescript(types: &mut crate::typescript::Types) -> String {
        crate::typescript::promise(&T::returns(types))
    }
}

/// A JavaScript `ReadableStream`, which Rust passes between the runtime's
/// functions without reading it.
#[derive(Clone)]
pub(crate) struct JsReadableStream<'js>(Object<'js>);

impl<'js> JsReadableStream<'js> {
    pub(crate) fn into_inner(self) -> Object<'js> {
        self.0
    }
}

impl<'js> FromJs<'js> for JsReadableStream<'js> {
    fn from_js(ctx: &Ctx<'js>, value: Value<'js>) -> rquickjs::Result<Self> {
        Object::from_js(ctx, value).map(Self)
    }
}

impl<'js> IntoJs<'js> for JsReadableStream<'js> {
    fn into_js(self, ctx: &Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        self.0.into_js(ctx)
    }
}

#[cfg(test)]
impl crate::typescript::JsType for JsReadableStream<'_> {
    fn typescript(_: &mut crate::typescript::Types) -> String {
        "ReadableStream".to_owned()
    }
}

/// Name and value pairs, such as headers, which JavaScript holds as an array
/// of two-element arrays.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct StringPairs(pub(crate) Vec<(String, String)>);

impl<'js> FromJs<'js> for StringPairs {
    fn from_js(ctx: &Ctx<'js>, value: Value<'js>) -> rquickjs::Result<Self> {
        let pairs = Vec::<List<(String, String)>>::from_js(ctx, value)?;
        Ok(Self(pairs.into_iter().map(|List(pair)| pair).collect()))
    }
}

impl<'js> IntoJs<'js> for StringPairs {
    fn into_js(self, ctx: &Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        let pairs: Vec<_> = self.0.into_iter().map(List).collect();
        pairs.into_js(ctx)
    }
}

#[cfg(test)]
impl crate::typescript::JsType for StringPairs {
    fn typescript(types: &mut crate::typescript::Types) -> String {
        Vec::<List<(String, String)>>::typescript(types)
    }
}
