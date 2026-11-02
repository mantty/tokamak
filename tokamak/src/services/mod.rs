mod lifetime;
mod streams;
pub(crate) use lifetime::{Lifetime, Scope};
pub(crate) use streams::{MessageCodec, StreamCodec, Streams};

use std::collections::BTreeMap;
use std::sync::Arc;

use rquickjs::class::Trace;
use rquickjs::{ArrayBuffer, Class, Ctx, Exception, FromJs, IntoJs, JsLifetime};
use tokamak_macros::JsType;

use crate::cancellation::Cancellation;
use crate::dispatcher::Bootstrap;
use crate::env_vars::ServiceBinding;
use crate::quickjs::{Error, RuntimeConfig, WorkerBundle};
use crate::typed::{JsPromise, JsReadableStream};

pub(crate) struct Call {
    pub(crate) entrypoint: String,
    pub(crate) method: String,
    pub(crate) arguments: Message,
}

#[derive(Debug)]
pub(crate) struct Service {
    pub(crate) bundle: WorkerBundle,
    pub(crate) config: RuntimeConfig,
    pub(crate) bindings: BTreeMap<String, ServiceBinding>,
}

#[derive(Debug)]
pub(crate) struct Registry {
    services: BTreeMap<String, Service>,
}

#[derive(Clone, Debug)]
pub(crate) struct Bindings {
    registry: Arc<Registry>,
    targets: BTreeMap<String, ServiceBinding>,
}

impl Registry {
    pub(crate) fn new(services: BTreeMap<String, Service>) -> Self {
        Self { services }
    }

    fn configuration(self: &Arc<Self>, service: &Service) -> RuntimeConfig {
        RuntimeConfig {
            services: Some(Bindings {
                registry: Arc::clone(self),
                targets: service.bindings.clone(),
            }),
            ..service.config.clone()
        }
    }

    pub(crate) fn worker(
        self: &Arc<Self>,
        name: &str,
    ) -> Result<(WorkerBundle, RuntimeConfig), Error> {
        let service = self
            .services
            .get(name)
            .ok_or_else(|| Error::Startup(format!("Missing service {name}")))?;
        Ok((service.bundle.clone(), self.configuration(service)))
    }

    pub(crate) async fn invoke(
        self: &Arc<Self>,
        target: ServiceBinding,
        method: String,
        arguments: Message,
        lifetime: Lifetime,
    ) -> Result<Reply, Error> {
        let registry = Arc::clone(self);
        let (sender, receiver) = flume::bounded(1);
        let cancellation = lifetime.clone();
        tokio::task::spawn_blocking(move || {
            let execution = async {
                let service = registry
                    .services
                    .get(&target.service)
                    .ok_or_else(|| Error::Engine(format!("Unknown service {}", target.service)))?;
                let config = registry.configuration(service);
                crate::dispatcher::execute_service(
                    &service.bundle,
                    &config,
                    Call {
                        entrypoint: target.entrypoint,
                        method,
                        arguments,
                    },
                    &lifetime,
                    &sender,
                )
                .await
            };
            let result = tokio::runtime::Handle::current().block_on(execution);
            if let Err(error) = result {
                let _ = sender.send(Err(error));
            }
        });
        tokio::select! {
            result = receiver.recv_async() => result.map_err(|error| Error::Engine(error.to_string()))?,
            () = cancellation.cancelled() => Err(Error::Engine("Service invocation was cancelled".into())),
        }
    }
}

/// Data independent of either JavaScript runtime; buffers are copied at the boundary.
#[derive(Debug)]
pub(crate) struct Message {
    pub(crate) data: String,
    buffers: Vec<Vec<u8>>,
    streams: Vec<streams::Port>,
}

impl Message {
    #[cfg(test)]
    fn empty_arguments() -> Self {
        Self {
            data: r#"{"root":["ref",0],"nodes":[["array",0,[]]]}"#.into(),
            buffers: Vec::new(),
            streams: Vec::new(),
        }
    }

    fn undefined() -> Self {
        Self {
            data: r#"{"root":["undefined"],"nodes":[]}"#.into(),
            buffers: Vec::new(),
            streams: Vec::new(),
        }
    }

    fn from_js<'js>(message: JsMessage<'js>, codec: &MessageCodec<'js>) -> rquickjs::Result<Self> {
        let buffers = message
            .buffers
            .iter()
            .map(|buffer| {
                buffer
                    .as_bytes()
                    .map(<[u8]>::to_vec)
                    .ok_or_else(|| Exception::throw_type(codec.ctx(), "Detached service buffer"))
            })
            .collect::<rquickjs::Result<_>>()?;
        let streams = message
            .streams
            .into_iter()
            .map(|stream| codec.take(stream))
            .collect::<rquickjs::Result<_>>()?;
        Ok(Self {
            data: message.data,
            buffers,
            streams,
        })
    }

    pub(crate) fn into_js<'js>(
        self,
        codec: &MessageCodec<'js>,
    ) -> rquickjs::Result<JsMessage<'js>> {
        let ctx = codec.ctx();
        let buffers = self
            .buffers
            .into_iter()
            .map(|buffer| ArrayBuffer::new(ctx.clone(), buffer))
            .collect::<rquickjs::Result<_>>()?;
        let streams = self
            .streams
            .into_iter()
            .map(|stream| codec.receive(stream))
            .collect::<rquickjs::Result<_>>()?;
        Ok(JsMessage {
            data: self.data,
            buffers,
            streams,
        })
    }
}

/// A service message as JavaScript holds it: its encoded value, and the
/// buffers and streams the value refers to.
#[derive(FromJs, IntoJs, JsType)]
pub(crate) struct JsMessage<'js> {
    data: String,
    buffers: Vec<ArrayBuffer<'js>>,
    streams: Vec<JsReadableStream<'js>>,
}

pub(crate) struct Reply {
    pub(crate) failed: bool,
    pub(crate) value: Message,
}

impl Reply {
    pub(crate) fn from_js<'js>(
        reply: JsReply<'js>,
        codec: &MessageCodec<'js>,
    ) -> rquickjs::Result<Self> {
        Ok(Self {
            failed: reply.failed,
            value: Message::from_js(reply.value, codec)?,
        })
    }

    fn into_js<'js>(self, codec: &MessageCodec<'js>) -> rquickjs::Result<JsReply<'js>> {
        Ok(JsReply {
            failed: self.failed,
            value: self.value.into_js(codec)?,
        })
    }
}

/// A reply as JavaScript holds it: the value or the error that failed the
/// call.
#[derive(FromJs, IntoJs, JsType)]
pub(crate) struct JsReply<'js> {
    failed: bool,
    value: JsMessage<'js>,
}

/// The Worker's service bindings, which `call` invokes.
#[derive(Trace, JsLifetime)]
#[rquickjs::class]
pub(crate) struct ServiceHost {
    #[qjs(skip_trace)]
    bindings: Bindings,
    #[qjs(skip_trace)]
    lifetime: Lifetime,
    #[qjs(skip_trace)]
    streams: Arc<Streams>,
}

#[tokamak_macros::js_methods]
#[rquickjs::methods]
impl ServiceHost {
    /// Calls `method` on the service bound as `name` with the arguments
    /// `message`, whose streams `codec` moves.
    #[allow(clippy::needless_pass_by_value)] // Native function arguments are owned by rquickjs.
    pub fn call<'js>(
        &self,
        ctx: Ctx<'js>,
        name: String,
        method: String,
        message: JsMessage<'js>,
        codec: StreamCodec<'js>,
    ) -> rquickjs::Result<ServiceCall<'js>> {
        let target = self
            .bindings
            .targets
            .get(&name)
            .cloned()
            .ok_or_else(|| Exception::throw_type(&ctx, "Unknown service binding"))?;
        let codec = MessageCodec::new(&ctx, codec, &self.streams);
        let message = Message::from_js(message, &codec)?;
        let registry = Arc::clone(&self.bindings.registry);
        let lifetime = Lifetime {
            caller: self.lifetime.caller.child_token(),
            ..self.lifetime.clone()
        };
        let cancellation = Cancellation::instance(&ctx, lifetime.caller.clone())?;
        let calling = ctx.clone();
        let reply = JsPromise::wrap_future(&ctx, async move {
            let result = registry
                .invoke(target, method, message, lifetime)
                .await
                .map_err(|error| crate::errors::throw_message(&calling, &error.to_string()))?;
            result.into_js(&codec)
        })?;
        Ok(ServiceCall {
            reply,
            cancellation,
        })
    }
}

/// A service call's reply, and the handle that cancels the call.
#[derive(IntoJs, JsType)]
pub(crate) struct ServiceCall<'js> {
    reply: JsPromise<'js, JsReply<'js>>,
    cancellation: Class<'js, Cancellation>,
}

pub(crate) fn install<'js>(
    ctx: &Ctx<'js>,
    bootstrap: &Bootstrap<'js>,
    bindings: &Bindings,
    scope: &Scope,
) -> rquickjs::Result<()> {
    let names: Vec<_> = bindings.targets.keys().cloned().collect();
    let host = ServiceHost {
        bindings: bindings.clone(),
        lifetime: scope.lifetime.clone(),
        streams: Arc::clone(&scope.streams),
    };
    bootstrap
        .install_services
        .call((names, Class::instance(ctx.clone(), host)?))
}

#[cfg(test)]
mod tests;
