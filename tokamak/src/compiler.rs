use rquickjs::loader::{ImportAttributes, Loader, Resolver};
use rquickjs::{
    CatchResultExt, Context, Ctx, Module, Runtime, WriteOptions, WriteOptionsEndianness,
};

/// Whether the module's source text is written into its bytecode.
#[allow(dead_code)] // build.rs only compiles stripped builtins
#[derive(Clone, Copy)]
pub(crate) enum SourceText {
    /// Source is retained, so `Function.prototype.toString` returns it.
    Embedded,
    /// Source is omitted; line numbers are kept for stack traces.
    Stripped,
}

pub(crate) fn compile_module(
    name: &str,
    source: &[u8],
    source_text: SourceText,
) -> Result<Vec<u8>, String> {
    let runtime = Runtime::new().map_err(|error| format!("runtime: {error}"))?;
    // Imports are linked by the runtime loader; compilation needs only declarations.
    runtime.set_loader(CompileResolver, CompileLoader);
    let context = Context::full(&runtime).map_err(|error| format!("context: {error}"))?;
    context.with(|ctx| {
        let module = Module::declare(ctx.clone(), name, source)
            .catch(&ctx)
            .map_err(|error| format!("declare: {error}"))?;
        module
            .write(WriteOptions {
                endianness: WriteOptionsEndianness::Little,
                strip_source: matches!(source_text, SourceText::Stripped),
                ..WriteOptions::default()
            })
            .map_err(|error| format!("write: {error}"))
    })
}

pub(crate) fn resolve_module_name(base: &str, name: &str) -> String {
    if !name.starts_with('.') {
        return name.to_owned();
    }
    let directory = base.rsplit_once('/').map_or("", |(directory, _)| directory);
    let mut parts = Vec::new();
    let combined = format!("{directory}/{name}");
    for part in combined.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if parts.last().is_some_and(|part| *part != "..") {
                    parts.pop();
                } else {
                    parts.push("..");
                }
            }
            part => parts.push(part),
        }
    }
    parts.join("/")
}

struct CompileResolver;

impl Resolver for CompileResolver {
    fn resolve<'js>(
        &mut self,
        _ctx: &Ctx<'js>,
        base: &str,
        name: &str,
        _attributes: Option<ImportAttributes<'js>>,
    ) -> rquickjs::Result<String> {
        Ok(resolve_module_name(base, name))
    }
}

struct CompileLoader;

impl Loader for CompileLoader {
    fn load<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        name: &str,
        _attributes: Option<ImportAttributes<'js>>,
    ) -> rquickjs::Result<Module<'js>> {
        Module::declare(ctx.clone(), name, b"export {};")
    }
}
