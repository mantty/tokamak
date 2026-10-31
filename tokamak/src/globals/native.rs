#![allow(clippy::needless_pass_by_value)]

use encoding_rs::{Decoder, DecoderResult, Encoding};
use rquickjs::class::Trace;
use rquickjs::module::{Declarations, Exports, ModuleDef};
use rquickjs::{
    ArrayBuffer, Class, Constructor, Ctx, Exception, Function, IntoJs, JsLifetime, Object,
    TypedArray, Value,
};
use std::io::Write;
use tokamak_macros::host_function;

use crate::runtime_modules;

pub(crate) struct HostModule;

tokamak_macros::host_functions! {
    pub(super),
    "asyncContextGet" => super::async_context::get,
    "asyncContextSet" => super::async_context::set,
    "createDecoder" => create_decoder,
    "randomBytes" => random_bytes,
    "detachArrayBuffer" => detach_array_buffer,
    "objectClass" => super::objects::class_name,
    "cloneArrayBuffer" => super::buffers::clone_array_buffer,
    "scheduleTimer" => super::timers::schedule_timer,
    "arrayBufferView" => super::buffers::array_buffer_view,
    "createCompression" => super::compression::create,
    "createNodeCompression" => super::compression::node::create,
    "htmlRewrite" => super::html_rewriter::rewrite_html,
    "htmlValidateSelector" => super::html_rewriter::validate_selector,
    "httpStatusText" => crate::network::http::status_text,
    "socketConnect" => crate::network::sockets::start,
    "ipVersion" => ip_version,
    "cacheMatch" => crate::cache::cache_match,
    "cachePut" => crate::cache::cache_put,
    "cacheDelete" => crate::cache::cache_delete,
    "writeStdout" => write_stdout,
    "writeStderr" => write_stderr,
    "registerDomException" => register_dom_exception,
    "builtinNamespace" => builtin_namespace,
    "isNodeBuiltin" => is_node_builtin,
    "nodeBuiltinNames" => runtime_modules::node_builtin_names,
    "processCwd" => crate::fs::node::exports::working_directory,
    "processChdir" => crate::fs::node::exports::change_directory,
}

/// The TypeScript signatures of this module's exports.
#[cfg(test)]
pub(crate) fn declarations(
    types: &mut crate::typescript::Types,
) -> Vec<(&'static str, crate::typescript::Signature)> {
    let mut declarations = host_declarations(types);
    declarations.extend(super::crypto::host_declarations(types));
    declarations.extend(super::intl::host_declarations(types));
    declarations.extend(super::url::host_declarations(types));
    declarations
}

/// The `DOMException` class native errors are created with, registered by the
/// module that defines it. A runtime has one context, so one class.
#[derive(JsLifetime)]
struct DomException<'js>(Constructor<'js>);

#[host_function]
fn register_dom_exception<'js>(
    ctx: Ctx<'js>,
    constructor: Constructor<'js>,
) -> rquickjs::Result<()> {
    ctx.store_userdata(DomException(constructor))
        .map(drop)
        .map_err(|_| Exception::throw_internal(&ctx, "DOMException could not be registered"))
}

/// A `DOMException` named `name`, thrown in `ctx`.
pub(super) fn throw_dom_exception(ctx: &Ctx<'_>, name: &str, message: &str) -> rquickjs::Error {
    let Some(constructor) = ctx.userdata::<DomException<'_>>() else {
        return Exception::throw_internal(ctx, &format!("{name}: {message}"));
    };
    match constructor.0.construct::<_, Object>((message, name)) {
        Ok(exception) => ctx.throw(exception.into()),
        Err(error) => error,
    }
}

/// Exports that `evaluate` constructs itself.
const CONSTRUCTED_EXPORTS: [&str; 1] = ["httpFetch"];

impl ModuleDef for HostModule {
    fn declare(exports: &Declarations) -> rquickjs::Result<()> {
        for name in CONSTRUCTED_EXPORTS
            .iter()
            .chain(HOST_EXPORTS)
            .chain(super::crypto::HOST_EXPORTS)
            .chain(super::intl::HOST_EXPORTS)
            .chain(super::url::HOST_EXPORTS)
        {
            exports.declare(*name)?;
        }
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> rquickjs::Result<()> {
        super::buffers::install(ctx);
        export_host_functions(ctx, exports)?;
        super::crypto::export_host_functions(ctx, exports)?;
        super::intl::export_host_functions(ctx, exports)?;
        super::url::export_host_functions(ctx, exports)?;
        let client = crate::network::http::client()
            .map_err(|error| Exception::throw_internal(ctx, &error.to_string()))?;
        exports.export(
            "httpFetch",
            Function::new(
                ctx.clone(),
                move |ctx: Ctx<'js>,
                      url: String,
                      method: String,
                      headers: String,
                      body: Value<'js>,
                      redirect: String| {
                    crate::network::http::start(ctx, &client, url, method, headers, body, redirect)
                },
            )?,
        )?;
        Ok(())
    }
}

#[host_function]
fn ip_version(input: String) -> u8 {
    crate::network::sockets::ip_version(&input)
}

#[host_function]
fn write_stdout(text: String) {
    write_flushed(&mut std::io::stdout().lock(), &text);
}

#[host_function]
fn write_stderr(text: String) {
    write_flushed(&mut std::io::stderr().lock(), &text);
}

/// A module's namespace object.
struct ModuleNamespace<'js>(Object<'js>);

impl<'js> IntoJs<'js> for ModuleNamespace<'js> {
    fn into_js(self, _: &Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        Ok(self.0.into_value())
    }
}

#[cfg(test)]
impl crate::typescript::JsType for ModuleNamespace<'_> {
    fn typescript(_: &mut crate::typescript::Types) -> String {
        "Record<string, unknown>".to_owned()
    }
}

#[host_function]
fn builtin_namespace(ctx: Ctx<'_>, name: String) -> rquickjs::Result<Option<ModuleNamespace<'_>>> {
    Ok(runtime_modules::builtin_namespace(ctx, &name)?.map(ModuleNamespace))
}

#[host_function]
fn is_node_builtin(name: String) -> bool {
    runtime_modules::is_node_builtin(&name)
}

fn write_flushed(output: &mut dyn Write, text: &str) {
    let _ = output.write_all(text.as_bytes());
    let _ = output.flush();
}

#[host_function]
fn create_decoder(
    ctx: Ctx<'_>,
    label: String,
    fatal: bool,
    ignore_bom: bool,
) -> rquickjs::Result<Class<'_, TextDecoderState>> {
    let encoding = Encoding::for_label_no_replacement(label.as_bytes())
        .ok_or_else(|| Exception::throw_range(&ctx, "Unsupported encoding"))?;
    let state = TextDecoderState {
        encoding,
        fatal,
        ignore_bom,
        decoder: new_decoder(encoding, ignore_bom),
    };
    Class::instance(ctx, state)
}

fn new_decoder(encoding: &'static Encoding, ignore_bom: bool) -> Decoder {
    if ignore_bom {
        encoding.new_decoder_without_bom_handling()
    } else {
        encoding.new_decoder_with_bom_removal()
    }
}

/// A `TextDecoder`'s encoding and stream state.
#[derive(Trace, JsLifetime)]
#[rquickjs::class]
pub(crate) struct TextDecoderState {
    #[qjs(skip_trace)]
    encoding: &'static Encoding,
    fatal: bool,
    ignore_bom: bool,
    #[qjs(skip_trace)]
    decoder: Decoder,
}

#[tokamak_macros::js_methods]
#[rquickjs::methods]
impl TextDecoderState {
    #[qjs(get)]
    pub fn encoding(&self) -> String {
        self.encoding.name().to_ascii_lowercase()
    }

    pub fn decode(&mut self, input: TypedArray<'_, u8>, stream: bool) -> rquickjs::Result<String> {
        let ctx = input.ctx();
        let bytes = input
            .as_bytes()
            .ok_or_else(|| Exception::throw_type(ctx, "Detached buffer"))?;
        let capacity = self
            .decoder
            .max_utf8_buffer_length(bytes.len())
            .ok_or_else(|| Exception::throw_range(ctx, "Input is too large"))?;
        let mut output = String::with_capacity(capacity);
        let malformed = if self.fatal {
            let (status, _) =
                self.decoder
                    .decode_to_string_without_replacement(bytes, &mut output, !stream);
            matches!(status, DecoderResult::Malformed(_, _))
        } else {
            let _ = self.decoder.decode_to_string(bytes, &mut output, !stream);
            false
        };
        let unicode = [
            encoding_rs::UTF_8,
            encoding_rs::UTF_16LE,
            encoding_rs::UTF_16BE,
        ]
        .contains(&self.encoding);
        // Workerd preserves Unicode stream state after a fatal error.
        if !stream || (malformed && !unicode) {
            self.decoder = new_decoder(self.encoding, self.ignore_bom);
        }
        if malformed {
            return Err(Exception::throw_type(ctx, "Invalid encoded data"));
        }
        Ok(output)
    }
}

#[host_function]
fn random_bytes(ctx: Ctx<'_>, length: usize) -> rquickjs::Result<TypedArray<'_, u8>> {
    if length > 65_536 {
        return Err(Exception::throw_range(
            &ctx,
            "Random data exceeds 65536 bytes",
        ));
    }
    let mut bytes = vec![0; length];
    getrandom::fill(&mut bytes)
        .map_err(|error| Exception::throw_internal(&ctx, &error.to_string()))?;
    TypedArray::new(ctx, bytes)
}

#[host_function]
fn detach_array_buffer(mut buffer: ArrayBuffer<'_>) {
    buffer.detach();
}
