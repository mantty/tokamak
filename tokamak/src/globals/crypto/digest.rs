use hmac::digest::DynDigest;
use hmac::{Hmac, KeyInit};
use md4::Md4;
use rquickjs::class::Trace;
use rquickjs::function::Opt;
use rquickjs::{ArrayBuffer, Class, Ctx, Exception, JsLifetime, TypedArray};

use super::hash::{DynHmac, MacState, Md5Sha1};

enum State {
    Hash(Box<dyn DynDigest>),
    Hmac(Box<dyn MacState>),
}

#[derive(Trace, JsLifetime)]
#[rquickjs::class]
pub(crate) struct DigestState {
    #[qjs(skip_trace)]
    state: Option<State>,
}

pub(super) fn create<'js>(
    ctx: Ctx<'js>,
    algorithm: String,
    Opt(key): Opt<TypedArray<'js, u8>>,
) -> rquickjs::Result<Class<'js, DigestState>> {
    let key = key
        .as_ref()
        .map(|key| {
            key.as_bytes()
                .ok_or_else(|| Exception::throw_type(&ctx, "Detached key"))
        })
        .transpose()?;
    let state = match (algorithm.as_str(), key) {
        ("MD4", Some(key)) => State::Hmac(Box::new(
            <Hmac<Md4> as KeyInit>::new_from_slice(key)
                .map_err(|error| Exception::throw_type(&ctx, &error.to_string()))?,
        )),
        ("MD4", None) => State::Hash(Box::new(Md4::default())),
        ("MD5-SHA1", Some(key)) => State::Hmac(Box::new(DynHmac::new(
            Box::new(Md5Sha1::default()),
            64,
            key,
        ))),
        ("MD5-SHA1", None) => State::Hash(Box::new(Md5Sha1::default())),
        (_, key) => {
            let hash = super::message_digest(&ctx, &algorithm)?;
            match key {
                Some(key) => State::Hmac(
                    hash.mac(key)
                        .map_err(|error| Exception::throw_message(&ctx, &error.to_string()))?,
                ),
                None => State::Hash(hash.hasher()),
            }
        }
    };
    Class::instance(ctx, DigestState { state: Some(state) })
}

#[rquickjs::methods]
impl DigestState {
    pub fn update(&mut self, input: TypedArray<'_, u8>) -> rquickjs::Result<()> {
        let ctx = input.ctx();
        let bytes = input
            .as_bytes()
            .ok_or_else(|| Exception::throw_type(ctx, "Detached digest input"))?;
        let state = self
            .state
            .as_mut()
            .ok_or_else(|| Exception::throw_message(ctx, "Digest already called"))?;
        match state {
            State::Hash(state) => state.update(bytes),
            State::Hmac(state) => state.update(bytes),
        }
        Ok(())
    }

    pub fn finish<'js>(&mut self, ctx: Ctx<'js>) -> rquickjs::Result<ArrayBuffer<'js>> {
        let state = self
            .state
            .take()
            .ok_or_else(|| Exception::throw_message(&ctx, "Digest already called"))?;
        let output = match state {
            State::Hash(state) => state.finalize().to_vec(),
            State::Hmac(state) => state.finalize(),
        };
        ArrayBuffer::new(ctx, output)
    }

    pub fn copy<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<Class<'js, Self>> {
        let Some(State::Hash(state)) = &self.state else {
            return Err(Exception::throw_type(&ctx, "Cannot copy this digest"));
        };
        Class::instance(
            ctx,
            Self {
                state: Some(State::Hash(state.box_clone())),
            },
        )
    }
}
