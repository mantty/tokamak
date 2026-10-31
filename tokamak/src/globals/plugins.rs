//! The Worker's native calls on the shell's plugins.

use std::future::Future;
use std::sync::Arc;

use rquickjs::class::Trace;
use rquickjs::{
    ArrayBuffer, Class, Ctx, Exception, FromJs, IntoJs, JsLifetime, Object, TypedArray, Value,
};
use tokamak_macros::JsType;
use tokio_util::sync::CancellationToken;

use crate::cancellation::Cancellation;
use crate::dispatcher::Bootstrap;
use crate::plugin_calls::{Body, Overflow, PluginCalls, PluginReply, Subscription, error};
use crate::typed::{JsFunction, JsPromise};

/// Gives the Worker its native calls and invocation-owned subscriptions.
pub(crate) fn install<'js>(
    ctx: &Ctx<'js>,
    bootstrap: &Bootstrap<'js>,
    calls: &Arc<PluginCalls>,
) -> rquickjs::Result<()> {
    let host = NativeHost {
        calls: Arc::clone(calls),
    };
    bootstrap
        .install_plugins
        .call((Class::instance(ctx.clone(), host)?, PLATFORM))
}

/// The shell's plugins, which the Worker calls and subscribes to.
#[derive(Trace, JsLifetime)]
#[rquickjs::class]
pub(crate) struct NativeHost {
    #[qjs(skip_trace)]
    calls: Arc<PluginCalls>,
}

#[tokamak_macros::js_methods]
#[rquickjs::methods]
impl NativeHost {
    /// Calls `method` on `plugin` with the encoded arguments `payload`, and
    /// with the body whose chunks `pull` resolves to.
    #[allow(clippy::needless_pass_by_value)] // Native function arguments are owned by rquickjs.
    pub fn call<'js>(
        &self,
        ctx: Ctx<'js>,
        plugin: String,
        method: String,
        payload: TypedArray<'js, u8>,
        pull: Option<JsFunction<'js, (), JsPromise<'js, BodyChunk<'js>>>>,
    ) -> rquickjs::Result<NativeCall<'js>> {
        let payload = payload
            .as_bytes()
            .ok_or_else(|| Exception::throw_type(&ctx, "Detached plugin payload"))?;
        let cancelled = CancellationToken::new();
        let reply = match pull {
            None => settle(&ctx, self.calls.call(&plugin, &method, payload), &cancelled)?,
            Some(pull) => {
                let (result, body) = self.calls.call_with_body(&plugin, &method, payload);
                if let Some(body) = body {
                    read_body(&ctx, body, pull);
                }
                settle(&ctx, result, &cancelled)?
            }
        };
        Ok(NativeCall {
            reply,
            cancellation: Cancellation::instance(&ctx, cancelled)?,
        })
    }

    /// Subscribes `subscriber` to `method` on `plugin` with the encoded
    /// arguments `payload`, until the returned handle is cancelled.
    /// Discarding the handle leaves the subscriber registered.
    #[allow(clippy::needless_pass_by_value)] // Native function arguments are owned by rquickjs.
    pub fn subscribe<'js>(
        &self,
        ctx: Ctx<'js>,
        plugin: String,
        method: String,
        payload: TypedArray<'js, u8>,
        subscriber: Subscriber<'js>,
    ) -> rquickjs::Result<Class<'js, Cancellation>> {
        let payload = payload
            .as_bytes()
            .ok_or_else(|| Exception::throw_type(&ctx, "Detached plugin payload"))?;
        let subscription = self
            .calls
            .subscribe(&plugin, &method, payload, subscriber.overflow);
        listen(&ctx, subscription, subscriber.deliver)
    }
}

/// A subscription's receiver: `deliver` takes each result, and `overflow`
/// handles a result that arrives while the queue is full.
#[derive(FromJs, JsType)]
pub(crate) struct Subscriber<'js> {
    deliver: JsFunction<'js, (PluginReply,)>,
    overflow: Overflow,
}

impl<'js> FromJs<'js> for Overflow {
    fn from_js(ctx: &Ctx<'js>, value: Value<'js>) -> rquickjs::Result<Self> {
        let name = String::from_js(ctx, value)?;
        Self::named(&name).ok_or_else(|| Exception::throw_type(ctx, "Unknown overflow policy"))
    }
}

#[cfg(test)]
impl crate::typescript::JsType for Overflow {
    fn typescript(types: &mut crate::typescript::Types) -> String {
        types.named("Overflow", std::any::type_name::<Self>(), |_| {
            crate::typescript::alias("Overflow", "\"error\" | \"dropOldest\"")
        })
    }
}

/// A native call's reply, and the handle that cancels the call.
#[derive(IntoJs, JsType)]
pub(crate) struct NativeCall<'js> {
    reply: JsPromise<'js, PluginReply>,
    cancellation: Class<'js, Cancellation>,
}

/// A chunk of a native call's body: its bytes, or its encoded error when
/// `failed`, and whether it is the last.
#[derive(FromJs, JsType)]
pub(crate) struct BodyChunk<'js> {
    payload: TypedArray<'js, u8>,
    failed: bool,
    done: bool,
}

/// The native platform the runtime runs on, as the Worker names it.
pub(crate) const PLATFORM: Platform = Platform(if cfg!(target_os = "ios") {
    "ios"
} else if cfg!(target_os = "android") {
    "android"
} else if cfg!(target_os = "windows") {
    "windows"
} else if cfg!(target_os = "macos") {
    "macos"
} else {
    "linux"
});

/// A native platform's name.
#[derive(Clone, Copy)]
pub(crate) struct Platform(pub(crate) &'static str);

impl<'js> IntoJs<'js> for Platform {
    fn into_js(self, ctx: &Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        self.0.into_js(ctx)
    }
}

#[cfg(test)]
impl crate::typescript::JsType for Platform {
    fn typescript(types: &mut crate::typescript::Types) -> String {
        types.named("NativePlatform", std::any::type_name::<Self>(), |_| {
            crate::typescript::alias(
                "NativePlatform",
                "\"ios\" | \"macos\" | \"android\" | \"windows\" | \"linux\"",
            )
        })
    }
}

/// The promise of `result`, which rejects with `AbortError` once
/// `cancelled`, cancelling the native work.
fn settle<'js>(
    ctx: &Ctx<'js>,
    result: impl Future<Output = PluginReply> + 'static,
    cancelled: &CancellationToken,
) -> rquickjs::Result<JsPromise<'js, PluginReply>> {
    let cancelled = cancelled.clone();
    JsPromise::wrap_future(ctx, async move {
        Ok(tokio::select! {
            result = result => result,
            () = cancelled.cancelled() => error("AbortError", "The native call was cancelled"),
        })
    })
}

/// Answers each of the shell's pulls on `body` with the chunk `pull`
/// resolves to, until the last or the end of the call.
fn read_body<'js>(
    ctx: &Ctx<'js>,
    mut body: Body,
    pull: JsFunction<'js, (), JsPromise<'js, BodyChunk<'js>>>,
) {
    ctx.spawn(async move {
        while body.pulled().await {
            let chunk = read_chunk(&pull)
                .await
                .unwrap_or_else(|_| error("DataCloneError", "The call's body could not be read"));
            body.send(&chunk);
            if chunk.done {
                return;
            }
        }
    });
}

async fn read_chunk<'js>(
    pull: &JsFunction<'js, (), JsPromise<'js, BodyChunk<'js>>>,
) -> rquickjs::Result<PluginReply> {
    let chunk = pull.call(())?.into_future().await?;
    Ok(PluginReply {
        payload: chunk.payload.as_bytes().unwrap_or_default().to_vec(),
        failed: chunk.failed,
        done: chunk.done,
    })
}

/// Passes each of `subscription`'s results to `deliver` until the last, or
/// until the returned handle is cancelled.
fn listen<'js>(
    ctx: &Ctx<'js>,
    mut subscription: Subscription,
    deliver: JsFunction<'js, (PluginReply,)>,
) -> rquickjs::Result<Class<'js, Cancellation>> {
    let removed = CancellationToken::new();
    let remove = removed.clone();
    let deliver = deliver.into_inner();
    ctx.spawn(async move {
        loop {
            let result = tokio::select! {
                biased;
                () = removed.cancelled() => return,
                result = subscription.next() => result,
            };
            let Some(result) = result else { return };
            super::run_task(&deliver, (result,), &removed);
        }
    });
    Cancellation::instance(ctx, remove)
}

impl<'js> IntoJs<'js> for PluginReply {
    fn into_js(self, ctx: &Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        let value = Object::new(ctx.clone())?;
        value.set("payload", ArrayBuffer::new(ctx.clone(), self.payload)?)?;
        value.set("failed", self.failed)?;
        value.set("done", self.done)?;
        Ok(value.into_value())
    }
}

/// A native call's result: its encoded value, or its encoded error when
/// `failed`, and whether it is the last.
#[cfg(test)]
impl crate::typescript::JsType for PluginReply {
    fn typescript(types: &mut crate::typescript::Types) -> String {
        types.named("PluginReply", std::any::type_name::<Self>(), |_| {
            crate::typescript::interface(
                "PluginReply",
                &[
                    ("payload", false, "ArrayBuffer".to_owned()),
                    ("failed", false, "boolean".to_owned()),
                    ("done", false, "boolean".to_owned()),
                ],
            )
        })
    }
}
