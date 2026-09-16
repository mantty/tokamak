use hmac::{Hmac, KeyInit, Mac};
use md4::{Digest, Md4};
use openssl::hash::{Hasher, MessageDigest};
use openssl::md::Md;
use openssl::md_ctx::MdCtx;
use openssl::pkey::{Id, PKey};
use rquickjs::class::Trace;
use rquickjs::function::Opt;
use rquickjs::{ArrayBuffer, Class, Ctx, Exception, JsLifetime, TypedArray};

enum State {
    Hash(Hasher),
    Hmac(MdCtx),
    // OpenSSL's default provider intentionally omits MD4. Use RustCrypto for
    // Workerd's legacy MD4 API without enabling an entire legacy provider.
    Md4(Md4),
    HmacMd4(Hmac<Md4>),
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
    let state = if algorithm == "MD4" {
        match key {
            Some(key) => State::HmacMd4(
                <Hmac<Md4> as KeyInit>::new_from_slice(key)
                    .map_err(|error| Exception::throw_type(&ctx, &error.to_string()))?,
            ),
            None => State::Md4(Md4::new()),
        }
    } else {
        let digest = match algorithm.as_str() {
            "SHA-224" => MessageDigest::sha224(),
            "MD5-SHA1" => MessageDigest::from_name("MD5-SHA1")
                .ok_or_else(|| Exception::throw_message(&ctx, "MD5-SHA1 is unavailable"))?,
            _ => super::message_digest(&ctx, &algorithm)?,
        };
        match key {
            Some(key) => {
                let create = || {
                    let key = PKey::private_key_from_raw_bytes(key, Id::HMAC)?;
                    let mut state = MdCtx::new()?;
                    let digest =
                        Md::from_nid(digest.type_()).ok_or_else(openssl::error::ErrorStack::get)?;
                    state.digest_sign_init(Some(digest), &key)?;
                    Ok::<_, openssl::error::ErrorStack>(state)
                };
                State::Hmac(
                    create().map_err(|error| Exception::throw_message(&ctx, &error.to_string()))?,
                )
            }
            None => State::Hash(
                Hasher::new(digest)
                    .map_err(|error| Exception::throw_message(&ctx, &error.to_string()))?,
            ),
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
            State::Hmac(state) => state.digest_sign_update(bytes),
            State::Md4(state) => {
                Digest::update(state, bytes);
                Ok(())
            }
            State::HmacMd4(state) => {
                Mac::update(state, bytes);
                Ok(())
            }
        }
        .map_err(|error| Exception::throw_message(ctx, &error.to_string()))
    }

    pub fn finish<'js>(&mut self, ctx: Ctx<'js>) -> rquickjs::Result<ArrayBuffer<'js>> {
        let state = self
            .state
            .take()
            .ok_or_else(|| Exception::throw_message(&ctx, "Digest already called"))?;
        let output = match state {
            State::Hash(mut state) => state.finish().map(|bytes| bytes.to_vec()),
            State::Hmac(mut state) => {
                let mut output = Vec::new();
                state.digest_sign_final_to_vec(&mut output).map(|_| output)
            }
            State::Md4(state) => Ok(state.finalize().to_vec()),
            State::HmacMd4(state) => Ok(state.finalize().into_bytes().to_vec()),
        }
        .map_err(|error| Exception::throw_message(&ctx, &error.to_string()))?;
        ArrayBuffer::new(ctx, output)
    }

    pub fn copy<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<Class<'js, Self>> {
        let state = match &self.state {
            Some(State::Hash(state)) => State::Hash(state.clone()),
            Some(State::Md4(state)) => State::Md4(state.clone()),
            _ => return Err(Exception::throw_type(&ctx, "Cannot copy this digest")),
        };
        Class::instance(ctx, Self { state: Some(state) })
    }
}
