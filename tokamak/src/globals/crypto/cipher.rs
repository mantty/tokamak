use openssl::symm::{Crypter, Mode};
use rquickjs::class::Trace;
use rquickjs::{ArrayBuffer, Class, Ctx, Exception, JsLifetime, TypedArray};

use super::{CipherKind, aes_cipher, bytes};

#[derive(Trace, JsLifetime)]
#[rquickjs::class]
pub(crate) struct CipherState {
    #[qjs(skip_trace)]
    crypter: Option<Crypter>,
    block_size: usize,
    authenticated: bool,
    encrypt: bool,
    tag_length: usize,
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
    let authenticated = mode == "gcm";
    let kind = match mode.as_str() {
        "cbc" => CipherKind::Cbc,
        "ctr" => CipherKind::Ctr,
        "gcm" => CipherKind::Gcm,
        _ => return Err(Exception::throw_type(&ctx, "Unsupported streaming cipher")),
    };
    let key = bytes(&ctx, key)?;
    let iv = bytes(&ctx, iv)?;
    let cipher = aes_cipher(&key, kind, &ctx)?;
    let mode = if encrypt {
        Mode::Encrypt
    } else {
        Mode::Decrypt
    };
    let crypter = Crypter::new(cipher, mode, &key, Some(&iv))
        .map_err(|error| Exception::throw_message(&ctx, &error.to_string()))?;
    Class::instance(
        ctx,
        CipherState {
            crypter: Some(crypter),
            block_size: cipher.block_size(),
            authenticated,
            encrypt,
            tag_length: tag_length.unwrap_or(16),
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
        let mut output = vec![0; input.len() + self.block_size];
        let crypter = self.active(&ctx)?;
        let count = crypter
            .update(&input, &mut output)
            .map_err(|error| Exception::throw_message(&ctx, &error.to_string()))?;
        output.truncate(count);
        ArrayBuffer::new(ctx, output)
    }

    pub fn finish<'js>(&mut self, ctx: Ctx<'js>) -> rquickjs::Result<ArrayBuffer<'js>> {
        let mut crypter = self.crypter.take().ok_or_else(|| finalized(&ctx))?;
        let mut output = vec![0; self.block_size];
        let count = crypter.finalize(&mut output).map_err(|error| {
            let message = if self.authenticated && !self.encrypt {
                "Authentication failed".to_owned()
            } else {
                error.to_string()
            };
            Exception::throw_message(&ctx, &message)
        })?;
        if self.authenticated && self.encrypt {
            let mut tag = vec![0; self.tag_length];
            crypter
                .get_tag(&mut tag)
                .map_err(|error| Exception::throw_message(&ctx, &error.to_string()))?;
            self.tag = Some(tag);
        }
        output.truncate(count);
        ArrayBuffer::new(ctx, output)
    }

    #[qjs(rename = "setAutoPadding")]
    pub fn set_auto_padding(&mut self, ctx: Ctx<'_>, padding: bool) -> rquickjs::Result<()> {
        self.active(&ctx)?.pad(padding);
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
            .aad_update(&data)
            .map_err(|error| Exception::throw_message(&ctx, &error.to_string()))
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
            .map_err(|error| Exception::throw_message(&ctx, &error.to_string()))?;
        self.tag_set = true;
        Ok(())
    }

    #[qjs(rename = "getAuthTag")]
    pub fn get_auth_tag<'js>(&mut self, ctx: Ctx<'js>) -> rquickjs::Result<ArrayBuffer<'js>> {
        if self.crypter.is_some() {
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
    fn active(&mut self, ctx: &Ctx<'_>) -> rquickjs::Result<&mut Crypter> {
        self.crypter.as_mut().ok_or_else(|| finalized(ctx))
    }
}

fn finalized(ctx: &Ctx<'_>) -> rquickjs::Error {
    Exception::throw_message(ctx, "Cipher/decipher context has already been finalized")
}
