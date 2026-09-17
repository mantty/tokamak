use rquickjs::class::Trace;
use rquickjs::{ArrayBuffer, Class, Ctx, Exception, JsLifetime, TypedArray};

use super::aes::{self, Mode, Stream};
use super::bytes;

#[derive(Trace, JsLifetime)]
#[rquickjs::class]
pub(crate) struct CipherState {
    #[qjs(skip_trace)]
    stream: Option<Box<dyn Stream>>,
    authenticated: bool,
    encrypt: bool,
    tag: Option<Vec<u8>>,
    tag_set: bool,
}

pub(super) fn create<'js>(
    ctx: Ctx<'js>,
    mode: String,
    encrypt: bool,
    key: TypedArray<'js, u8>,
    iv: TypedArray<'js, u8>,
    tag_length: Option<usize>,
) -> rquickjs::Result<Class<'js, CipherState>> {
    let mode = match mode.as_str() {
        "cbc" => Mode::Cbc,
        "ctr" => Mode::Ctr,
        "gcm" => Mode::Gcm,
        _ => return Err(Exception::throw_type(&ctx, "Unsupported streaming cipher")),
    };
    let key = bytes(&ctx, key)?;
    let iv = bytes(&ctx, iv)?;
    let stream = aes::stream(mode, encrypt, &key, &iv, tag_length.unwrap_or(16))
        .map_err(|error| Exception::throw_message(&ctx, error.0))?;
    Class::instance(
        ctx,
        CipherState {
            stream: Some(stream),
            authenticated: mode == Mode::Gcm,
            encrypt,
            tag: None,
            tag_set: false,
        },
    )
}

#[rquickjs::methods]
impl CipherState {
    pub fn update<'js>(
        &mut self,
        ctx: Ctx<'js>,
        input: TypedArray<'js, u8>,
    ) -> rquickjs::Result<ArrayBuffer<'js>> {
        let input = bytes(&ctx, input)?;
        let output = self
            .active(&ctx)?
            .update(&input)
            .map_err(|error| Exception::throw_message(&ctx, error.0))?;
        ArrayBuffer::new(ctx, output)
    }

    pub fn finish<'js>(&mut self, ctx: Ctx<'js>) -> rquickjs::Result<ArrayBuffer<'js>> {
        let stream = self.stream.take().ok_or_else(|| finalized(&ctx))?;
        let finished = stream.finish().map_err(|error| {
            let message = if self.authenticated && !self.encrypt {
                "Authentication failed"
            } else {
                error.0
            };
            Exception::throw_message(&ctx, message)
        })?;
        if self.authenticated && self.encrypt {
            self.tag = finished.tag;
        }
        ArrayBuffer::new(ctx, finished.output)
    }

    #[qjs(rename = "setAutoPadding")]
    pub fn set_auto_padding(&mut self, ctx: Ctx<'_>, padding: bool) -> rquickjs::Result<()> {
        self.active(&ctx)?.set_padding(padding);
        Ok(())
    }

    #[qjs(rename = "setAAD")]
    pub fn set_aad<'js>(
        &mut self,
        ctx: Ctx<'js>,
        data: TypedArray<'js, u8>,
    ) -> rquickjs::Result<()> {
        if !self.authenticated {
            return Err(Exception::throw_message(
                &ctx,
                "Cipher does not support AAD",
            ));
        }
        let data = bytes(&ctx, data)?;
        self.active(&ctx)?
            .set_aad(&data)
            .map_err(|error| Exception::throw_message(&ctx, error.0))
    }

    #[qjs(rename = "setAuthTag")]
    pub fn set_auth_tag<'js>(
        &mut self,
        ctx: Ctx<'js>,
        tag: TypedArray<'js, u8>,
    ) -> rquickjs::Result<()> {
        if self.tag_set {
            return Err(Exception::throw_message(&ctx, "Auth tag is already set"));
        }
        let tag = bytes(&ctx, tag)?;
        self.active(&ctx)?
            .set_tag(&tag)
            .map_err(|error| Exception::throw_message(&ctx, error.0))?;
        self.tag_set = true;
        Ok(())
    }

    #[qjs(rename = "getAuthTag")]
    pub fn get_auth_tag<'js>(&mut self, ctx: Ctx<'js>) -> rquickjs::Result<ArrayBuffer<'js>> {
        if self.stream.is_some() {
            return Err(Exception::throw_message(
                &ctx,
                "Auth tag is only available once cipher context has been finalized",
            ));
        }
        if !self.encrypt {
            return Err(Exception::throw_message(
                &ctx,
                "Getting the auth tag is only support for cipher",
            ));
        }
        ArrayBuffer::new(ctx, self.tag.take().unwrap_or_default())
    }

    #[qjs(skip)]
    fn active(&mut self, ctx: &Ctx<'_>) -> rquickjs::Result<&mut Box<dyn Stream>> {
        self.stream.as_mut().ok_or_else(|| finalized(ctx))
    }
}

fn finalized(ctx: &Ctx<'_>) -> rquickjs::Error {
    Exception::throw_message(ctx, "Cipher/decipher context has already been finalized")
}
