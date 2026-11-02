//! The TypeScript types of the Rust values tokamak hands to JavaScript and of
//! the JSON Rust exchanges with Node, and `src/host.generated.d.ts`, the
//! declarations generated from them that the runtime's TypeScript is checked
//! against.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::path::PathBuf;

use rquickjs::function::{Opt, Rest, This};
use rquickjs::prelude::List;
use rquickjs::{
    Array, ArrayBuffer, BigInt, Coerced, Constructor, Ctx, Exception, Function, Null, Object,
    Promise, Symbol, TypedArray, Undefined, Value,
};

/// A Rust value's JavaScript form, as a TypeScript type.
pub trait JsType {
    /// Whether the value may be missing, which makes a property of this type
    /// optional.
    const OPTIONAL: bool = false;

    /// Whether JavaScript may give `undefined` for the value.
    const ACCEPTS_UNDEFINED: bool = Self::OPTIONAL;

    /// The TypeScript type, after adding the named types it uses to `types`.
    fn typescript(types: &mut Types) -> String;
}

/// A host function parameter's TypeScript form.
pub trait JsParameter {
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
        // rquickjs reads an omitted argument as `None` but converts an explicit
        // `undefined` to `T`, which `name?:` allows.
        const {
            assert!(
                T::ACCEPTS_UNDEFINED,
                "an Opt parameter's type must accept undefined: use Opt<Option<T>>"
            );
        }
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
pub trait JsReturn {
    /// The result type, after adding the named types it uses to `types`.
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

/// A function's parameters, given as a tuple of their types.
pub trait JsArguments {
    /// Each parameter's declaration, after adding the named types they use to
    /// `types`.
    fn parameters(types: &mut Types) -> Vec<String>;
}

macro_rules! arguments {
    ($($argument:ident $index:literal),*) => {
        impl<$($argument: JsType),*> JsArguments for ($($argument,)*) {
            #[allow(unused_variables)]
            fn parameters(types: &mut Types) -> Vec<String> {
                vec![$(format!("arg{}: {}", $index, $argument::typescript(types))),*]
            }
        }
    };
}

arguments!();
arguments!(A 0);
arguments!(A 0, B 1);
arguments!(A 0, B 1, C 2);
arguments!(A 0, B 1, C 2, D 3);

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

impl JsType for Value<'_> {
    const ACCEPTS_UNDEFINED: bool = true;

    fn typescript(_: &mut Types) -> String {
        "unknown".to_owned()
    }
}

/// serde writes a path as a string.
impl JsType for PathBuf {
    fn typescript(_: &mut Types) -> String {
        "string".to_owned()
    }
}

impl JsType for serde_json::Value {
    fn typescript(_: &mut Types) -> String {
        "unknown".to_owned()
    }
}

impl JsType for serde_json::Map<String, serde_json::Value> {
    fn typescript(_: &mut Types) -> String {
        "Record<string, unknown>".to_owned()
    }
}

/// A value converted with JavaScript's coercion rules accepts any value.
impl<T> JsType for Coerced<T> {
    fn typescript(_: &mut Types) -> String {
        "unknown".to_owned()
    }
}

/// `None` is a missing property, as rquickjs converts it and serde reads it;
/// a serialized `Option` field skips `None` to match.
impl<T: JsType> JsType for Option<T> {
    const OPTIONAL: bool = true;

    fn typescript(types: &mut Types) -> String {
        format!("{} | undefined", union_member(&T::typescript(types)))
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

impl<V: JsType> JsType for BTreeMap<u32, V> {
    fn typescript(types: &mut Types) -> String {
        format!("Record<number, {}>", V::typescript(types))
    }
}

impl<V: JsType, S> JsType for HashMap<String, V, S> {
    fn typescript(types: &mut Types) -> String {
        format!("Record<string, {}>", V::typescript(types))
    }
}

/// rquickjs converts a `List` of a tuple to and from an array.
impl<A: JsType, B: JsType> JsType for List<(A, B)> {
    fn typescript(types: &mut Types) -> String {
        tuple(&[A::typescript(types), B::typescript(types)])
    }
}

/// `typescript` as one member of a union: a function type is parenthesised,
/// since its result type would otherwise take in the union.
fn union_member(typescript: &str) -> String {
    if typescript.contains("=>") {
        format!("({typescript})")
    } else {
        typescript.to_owned()
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
#[must_use]
pub fn parameters<const N: usize>(parameters: [Option<String>; N]) -> Vec<String> {
    parameters.into_iter().flatten().collect()
}

/// A promise of `output`.
#[must_use]
pub fn promise(output: &str) -> String {
    format!("Promise<{output}>")
}

/// A tuple of `elements`.
#[must_use]
pub fn tuple(elements: &[String]) -> String {
    format!("[{}]", elements.join(", "))
}

/// An interface declaration named `name` with `properties`: each one's name,
/// whether it is optional, and its type.
#[must_use]
pub fn interface(name: &str, properties: &[(&str, bool, String)]) -> String {
    let mut declaration = format!("export interface {name} {{\n");
    for property in properties {
        let _ = writeln!(declaration, "  {};", member(property));
    }
    declaration.push('}');
    declaration
}

/// An object type with `properties`, given as `interface` takes them.
pub fn object(properties: &[(&str, bool, String)]) -> String {
    let members: Vec<_> = properties.iter().map(member).collect();
    format!("{{ {} }}", members.join("; "))
}

/// A property's member declaration.
fn member((property, optional, typescript): &(&str, bool, String)) -> String {
    let optional = if *optional { "?" } else { "" };
    format!("{property}{optional}: {typescript}")
}

/// A type alias declaration named `name` for `typescript`.
#[must_use]
pub fn alias(name: &str, typescript: &str) -> String {
    format!("export type {name} = {typescript};")
}

/// The named types one declared module uses, by TypeScript name.
#[derive(Default)]
pub struct Types {
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
    /// `name`, after declaring it with `declare` if it is new.
    ///
    /// # Panics
    ///
    /// Panics when another Rust type has declared `name`: two Rust types may
    /// not share a TypeScript name.
    pub fn named(
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

    /// A module that exports the declared types and nothing else.
    #[must_use]
    pub fn exports(&self) -> String {
        let declarations: Vec<_> = self
            .declarations
            .values()
            .map(|declared| declared.typescript.as_str())
            .collect();
        format!("{}\n", declarations.join("\n\n"))
    }

    /// `name`, after declaring it if it is new as the interface of an rquickjs
    /// class with `members`, its methods and getters. A property keyed by a
    /// symbol only the declarations name keeps TypeScript code from building
    /// one or passing one class for another.
    pub fn class(
        &mut self,
        name: &str,
        rust: &'static str,
        members: impl FnOnce(&mut Self) -> Vec<String>,
    ) -> String {
        self.branded = true;
        self.named(name, rust, |types| {
            let mut declaration =
                format!("export interface {name} {{\n  readonly [classBrand]: {name:?};\n");
            for member in members(types) {
                let _ = writeln!(declaration, "  {member};");
            }
            declaration.push('}');
            declaration
        })
    }
}

/// A function's TypeScript parameters and result type.
pub struct Signature {
    parameters: Vec<String>,
    returns: String,
}

impl Signature {
    /// A signature of `parameters`, each declared, and the `returns` type.
    #[must_use]
    pub fn new(parameters: Vec<String>, returns: String) -> Self {
        Self {
            parameters,
            returns,
        }
    }

    /// The parameter list and result type, as a function declaration ends.
    fn declaration(&self) -> String {
        format!("({}): {}", self.parameters.join(", "), self.returns)
    }

    /// An interface member for a method named `name` with this signature.
    #[must_use]
    pub fn method(&self, name: &str) -> String {
        format!("{name}{}", self.declaration())
    }
}

/// An interface member for a getter named `name` returning `returns`.
#[must_use]
pub fn getter(name: &str, returns: &str) -> String {
    format!("readonly {name}: {returns}")
}

/// One declared module: its exported functions and the types they use.
#[cfg(all(test, feature = "native"))]
struct Module {
    name: &'static str,
    functions: Vec<(&'static str, Signature)>,
    types: Types,
}

#[cfg(all(test, feature = "native"))]
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
#[cfg(all(test, feature = "native"))]
fn declarations() -> String {
    let modules = [
        Module::new("tokamak:host", |types| {
            // `builtins/runtime.ts` exports the bootstrap that Rust reads.
            crate::dispatcher::Bootstrap::typescript(types);
            crate::fs::node::declare_types(types);
            crate::globals::native::declarations(types)
        }),
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

#[cfg(all(test, feature = "native"))]
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
