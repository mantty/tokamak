mod lifetime;
mod streams;
pub(crate) use lifetime::{Lifetime, Scope};
pub(crate) use streams::Streams;

use std::collections::BTreeMap;
use std::sync::Arc;

use rquickjs::{Array, ArrayBuffer, Ctx, Exception, Function, Object, Promise};

use crate::env_vars::ServiceBinding;
use crate::quickjs::{Error, RuntimeConfig, WorkerBundle};

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

    fn from_js<'js>(
        object: &Object<'js>,
        bootstrap: &Object<'js>,
        channels: &Arc<Streams>,
    ) -> rquickjs::Result<Self> {
        let buffers: Array = object.get("buffers")?;
        let buffers = buffers
            .iter::<ArrayBuffer>()
            .map(|buffer| {
                let buffer = buffer?;
                buffer
                    .as_bytes()
                    .map(<[u8]>::to_vec)
                    .ok_or_else(|| Exception::throw_type(object.ctx(), "Detached service buffer"))
            })
            .collect::<rquickjs::Result<_>>()?;
        let streams: Array = object.get("streams")?;
        let streams = streams
            .iter::<Object>()
            .map(|stream| channels.take(stream?, bootstrap))
            .collect::<rquickjs::Result<_>>()?;
        Ok(Self {
            data: object.get("data")?,
            buffers,
            streams,
        })
    }

    pub(crate) fn into_js<'js>(
        self,
        ctx: &Ctx<'js>,
        bootstrap: &Object<'js>,
        channels: &Arc<Streams>,
    ) -> rquickjs::Result<Object<'js>> {
        let object = Object::new(ctx.clone())?;
        let buffers = Array::new(ctx.clone())?;
        for (index, buffer) in self.buffers.into_iter().enumerate() {
            buffers.set(index, ArrayBuffer::new(ctx.clone(), buffer)?)?;
        }
        object.set("data", self.data)?;
        object.set("buffers", buffers)?;
        let streams = Array::new(ctx.clone())?;
        for (index, stream) in self.streams.into_iter().enumerate() {
            streams.set(index, channels.receive(ctx, stream, bootstrap)?)?;
        }
        object.set("streams", streams)?;
        Ok(object)
    }
}

pub(crate) struct Reply {
    pub(crate) failed: bool,
    pub(crate) value: Message,
}

impl Reply {
    pub(crate) fn from_js<'js>(
        object: &Object<'js>,
        bootstrap: &Object<'js>,
        channels: &Arc<Streams>,
    ) -> rquickjs::Result<Self> {
        Ok(Self {
            failed: object.get("failed")?,
            value: Message::from_js(&object.get("value")?, bootstrap, channels)?,
        })
    }

    fn into_js<'js>(
        self,
        ctx: &Ctx<'js>,
        bootstrap: &Object<'js>,
        channels: &Arc<Streams>,
    ) -> rquickjs::Result<Object<'js>> {
        let object = Object::new(ctx.clone())?;
        object.set("failed", self.failed)?;
        object.set("value", self.value.into_js(ctx, bootstrap, channels)?)?;
        Ok(object)
    }
}

pub(crate) fn install<'js>(
    ctx: &Ctx<'js>,
    bootstrap: &Object<'js>,
    bindings: &Bindings,
    scope: &Scope,
) -> rquickjs::Result<()> {
    let names: Vec<_> = bindings.targets.keys().cloned().collect();
    let bindings = bindings.clone();
    let lifetime = scope.lifetime.clone();
    let channels = Arc::clone(&scope.streams);
    let call = Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>,
              name: String,
              method: String,
              arguments: Object<'js>,
              bootstrap: Object<'js>| {
            let target = bindings
                .targets
                .get(&name)
                .cloned()
                .ok_or_else(|| Exception::throw_type(&ctx, "Unknown service binding"))?;
            let message = Message::from_js(&arguments, &bootstrap, &channels)?;
            let registry = Arc::clone(&bindings.registry);
            let lifetime = Lifetime {
                caller: lifetime.caller.child_token(),
                ..lifetime.clone()
            };
            let cancellation = lifetime.caller.clone();
            let calling = ctx.clone();
            let channels = Arc::clone(&channels);
            let result = Promise::wrap_future(&ctx, async move {
                let result = registry
                    .invoke(target, method, message, lifetime)
                    .await
                    .map_err(|error| crate::errors::throw_message(&calling, &error.to_string()))?;
                result.into_js(&calling, &bootstrap, &channels)
            })?;
            let operation = Object::new(ctx.clone())?;
            operation.set("result", result)?;
            operation.set("cancel", Function::new(ctx, move || cancellation.cancel())?)?;
            Ok::<_, rquickjs::Error>(operation)
        },
    )?;
    bootstrap
        .get::<_, Function>("installServices")?
        .call((names, call))
}

#[cfg(test)]
mod tests;
