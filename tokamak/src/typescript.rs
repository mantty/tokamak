//! The TypeScript types of the Rust values tokamak hands to JavaScript, and
//! `src/host.generated.d.ts`, the declarations generated from them that the
//! runtime's TypeScript is checked against.

use std::collections::{BTreeMap, HashMap};

use rquickjs::function::{Opt, Rest, This};
use rquickjs::{
    Array, ArrayBuffer, BigInt, Coerced, Constructor, Ctx, Exception, Function, Null, Object,
    Promise, Symbol, TypedArray, Undefined, Value,
};

/// A Rust value's JavaScript form, as a TypeScript type.
pub(crate) trait JsType {
    /// Whether the value may be missing, which makes a property of this type
    /// optional.
    const OPTIONAL: bool = false;

    /// The TypeScript type, after adding the named types it uses to `types`.
    fn typescript(types: &mut Types) -> String;
}

/// A host function parameter's TypeScript form.
pub(crate) trait JsParameter {
    /// The parameter named `name`, or `None` for a value rquickjs supplies
    /// rather than the caller, such as the context.
    fn parameter(name: &str, types: &mut Types) -> Option<String>;
}

impl<T: JsType> JsParameter for T {
    fn parameter(name: &str, types: &mut Types) -> Option<String> {
        Some(format!("{name}: {}", T::typescript(types)))
    }
}

impl JsParameter for Ctx<'_> {
    fn parameter(_: &str, _: &mut Types) -> Option<String> {
        None
    }
}

impl<T: JsType> JsParameter for Opt<T> {
    fn parameter(name: &str, types: &mut Types) -> Option<String> {
        Some(format!("{name}?: {}", T::typescript(types)))
    }
}

impl<T: JsType> JsParameter for Rest<T> {
    fn parameter(name: &str, types: &mut Types) -> Option<String> {
        Some(format!("...{name}: {}", array(&T::typescript(types))))
    }
}

impl<T: JsType> JsParameter for This<T> {
    fn parameter(_: &str, types: &mut Types) -> Option<String> {
        Some(format!("this: {}", T::typescript(types)))
    }
}

/// A function result's TypeScript type.
pub(crate) trait JsReturn {
    fn returns(types: &mut Types) -> String;
}

impl<T: JsType> JsReturn for T {
    fn returns(types: &mut Types) -> String {
        T::typescript(types)
    }
}

impl JsReturn for () {
    fn returns(_: &mut Types) -> String {
        "void".to_owned()
    }
}

impl<T: JsReturn> JsReturn for rquickjs::Result<T> {
    fn returns(types: &mut Types) -> String {
        T::returns(types)
    }
}

/// Types whose TypeScript form is fixed.
macro_rules! fixed_types {
    ($($rust:ty => $typescript:literal),+ $(,)?) => {
        $(impl JsType for $rust {
            fn typescript(_: &mut Types) -> String {
                $typescript.to_owned()
            }
        })+
    };
}

fixed_types! {
    bool => "boolean",
    char => "string",
    String => "string",
    &str => "string",
    i8 => "number",
    i16 => "number",
    i32 => "number",
    i64 => "number",
    isize => "number",
    u8 => "number",
    u16 => "number",
    u32 => "number",
    u64 => "number",
    usize => "number",
    f32 => "number",
    f64 => "number",
    Value<'_> => "unknown",
    Object<'_> => "object",
    Array<'_> => "unknown[]",
    Function<'_> => "(...args: never[]) => unknown",
    Constructor<'_> => "abstract new (...args: never[]) => unknown",
    Promise<'_> => "Promise<unknown>",
    ArrayBuffer<'_> => "ArrayBuffer",
    rquickjs::String<'_> => "string",
    Symbol<'_> => "symbol",
    BigInt<'_> => "bigint",
    Exception<'_> => "Error",
    Null => "null",
    Undefined => "undefined",
    TypedArray<'_, i8> => "Int8Array",
    TypedArray<'_, u8> => "Uint8Array",
    TypedArray<'_, i16> => "Int16Array",
    TypedArray<'_, u16> => "Uint16Array",
    TypedArray<'_, i32> => "Int32Array",
    TypedArray<'_, u32> => "Uint32Array",
    TypedArray<'_, f32> => "Float32Array",
    TypedArray<'_, f64> => "Float64Array",
    TypedArray<'_, i64> => "BigInt64Array",
    TypedArray<'_, u64> => "BigUint64Array",
}

/// A value converted with JavaScript's coercion rules accepts any value.
impl<T> JsType for Coerced<T> {
    fn typescript(_: &mut Types) -> String {
        "unknown".to_owned()
    }
}

impl<T: JsType> JsType for Option<T> {
    const OPTIONAL: bool = true;

    fn typescript(types: &mut Types) -> String {
        format!("{} | undefined", T::typescript(types))
    }
}

impl<T: JsType> JsType for Box<T> {
    fn typescript(types: &mut Types) -> String {
        T::typescript(types)
    }
}

impl<T: JsType> JsType for Vec<T> {
    fn typescript(types: &mut Types) -> String {
        array(&T::typescript(types))
    }
}

impl<V: JsType> JsType for BTreeMap<String, V> {
    fn typescript(types: &mut Types) -> String {
        format!("Record<string, {}>", V::typescript(types))
    }
}

impl<V: JsType, S> JsType for HashMap<String, V, S> {
    fn typescript(types: &mut Types) -> String {
        format!("Record<string, {}>", V::typescript(types))
    }
}

/// An array of `element`, parenthesised when `element` is not a single name.
fn array(element: &str) -> String {
    if element.chars().all(|c| c.is_alphanumeric() || c == '_') {
        format!("{element}[]")
    } else {
        format!("Array<{element}>")
    }
}

/// The parameters a function declares, without those its caller does not
/// pass.
pub(crate) fn parameters<const N: usize>(parameters: [Option<String>; N]) -> Vec<String> {
    parameters.into_iter().flatten().collect()
}

/// A promise of `output`.
pub(crate) fn promise(output: &str) -> String {
    format!("Promise<{output}>")
}

/// A tuple of `elements`.
pub(crate) fn tuple(elements: &[String]) -> String {
    format!("[{}]", elements.join(", "))
}

/// An interface declaration named `name` with `properties`: each one's name,
/// whether it is optional, and its type.
pub(crate) fn interface(name: &str, properties: &[(&str, bool, String)]) -> String {
    let mut declaration = format!("export interface {name} {{\n");
    for (property, optional, typescript) in properties {
        let optional = if *optional { "?" } else { "" };
        declaration.push_str(&format!("  {property}{optional}: {typescript};\n"));
    }
    declaration.push('}');
    declaration
}

/// A type alias declaration named `name` for `typescript`.
pub(crate) fn alias(name: &str, typescript: &str) -> String {
    format!("export type {name} = {typescript};")
}

/// The named types one declared module uses, by TypeScript name.
#[derive(Default)]
pub(crate) struct Types {
    declarations: BTreeMap<String, Declaration>,
    /// Whether a class uses `classBrand`.
    branded: bool,
}

struct Declaration {
    /// The Rust type named, which tells apart two types given one name.
    rust: &'static str,
    typescript: String,
}

impl Types {
    /// `name`, after declaring it with `declare` if it is new. Two Rust types
    /// may not share a TypeScript name.
    pub(crate) fn named(
        &mut self,
        name: &str,
        rust: &'static str,
        declare: impl FnOnce(&mut Self) -> String,
    ) -> String {
        if let Some(declared) = self.declarations.get(name) {
            assert_eq!(
                declared.rust, rust,
                "two Rust types are named {name} in TypeScript"
            );
            return name.to_owned();
        }
        // Declared before its fields, so a type that contains itself ends.
        let declaration = Declaration {
            rust,
            typescript: String::new(),
        };
        self.declarations.insert(name.to_owned(), declaration);
        let typescript = declare(self);
        if let Some(declared) = self.declarations.get_mut(name) {
            declared.typescript = typescript;
        }
        name.to_owned()
    }

    /// `name`, after declaring it if it is new as the interface of an rquickjs
    /// class with `methods`, each one's name and signature. A property keyed
    /// by a symbol only the declarations name keeps TypeScript code from
    /// building one or passing one class for another.
    pub(crate) fn class(
        &mut self,
        name: &str,
        rust: &'static str,
        methods: impl FnOnce(&mut Self) -> Vec<(&'static str, Signature)>,
    ) -> String {
        self.branded = true;
        self.named(name, rust, |types| {
            let mut declaration =
                format!("export interface {name} {{\n  readonly [classBrand]: {name:?};\n");
            for (method, signature) in methods(types) {
                declaration.push_str(&format!("  {method}{};\n", signature.declaration()));
            }
            declaration.push('}');
            declaration
        })
    }
}

/// A function's TypeScript parameters and result type.
pub(crate) struct Signature {
    parameters: Vec<String>,
    returns: String,
}

impl Signature {
    pub(crate) fn new(parameters: Vec<String>, returns: String) -> Self {
        Self {
            parameters,
            returns,
        }
    }

    /// The parameter list and result type, as a function declaration ends.
    fn declaration(&self) -> String {
        format!("({}): {}", self.parameters.join(", "), self.returns)
    }
}

/// One declared module: its exported functions and the types they use.
#[cfg(feature = "native")]
struct Module {
    name: &'static str,
    functions: Vec<(&'static str, Signature)>,
    types: Types,
}

#[cfg(feature = "native")]
impl Module {
    fn new(
        name: &'static str,
        declarations: fn(&mut Types) -> Vec<(&'static str, Signature)>,
    ) -> Self {
        let mut types = Types::default();
        let mut functions = declarations(&mut types);
        functions.sort_by_key(|(name, _)| *name);
        Self {
            name,
            functions,
            types,
        }
    }

    fn declaration(&self) -> String {
        let mut members = Vec::new();
        if self.types.branded {
            members.push("const classBrand: unique symbol;".to_owned());
        }
        for (name, signature) in &self.functions {
            members.push(format!(
                "export function {name}{};",
                signature.declaration()
            ));
        }
        members.extend(
            self.types
                .declarations
                .values()
                .map(|declared| declared.typescript.clone()),
        );
        let body = members.join("\n\n").replace('\n', "\n  ");
        format!("declare module {:?} {{\n  {body}\n}}\n", self.name)
    }
}

/// The contents of `src/host.generated.d.ts`.
#[cfg(feature = "native")]
fn declarations() -> String {
    let modules = [
        Module::new("tokamak:host", crate::globals::native::declarations),
        Module::new("tokamak:storage", crate::storage::declarations),
    ];
    let mut file = String::from(concat!(
        "// Generated from tokamak's Rust code; do not edit. Regenerate with\n",
        "// TOKAMAK_GENERATE=1 cargo test -p tokamak --features native generated_declarations\n",
    ));
    for module in modules {
        file.push('\n');
        file.push_str(&module.declaration().replace("\n  \n", "\n\n"));
    }
    file
}

#[cfg(feature = "native")]
mod tests {
    use std::path::Path;

    #[test]
    fn generated_declarations_are_current() -> std::io::Result<()> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.generated.d.ts");
        let generated = super::declarations();
        if std::env::var_os("TOKAMAK_GENERATE").is_some() {
            std::fs::write(&path, &generated)?;
        }
        let committed = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(
            committed.replace("\r\n", "\n") == generated,
            "src/host.generated.d.ts is out of date; regenerate it with \
             TOKAMAK_GENERATE=1 cargo test -p tokamak --features native generated_declarations"
        );
        Ok(())
    }
}
