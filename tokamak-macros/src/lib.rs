//! Macros that describe the Rust values tokamak hands to JavaScript, and the
//! JSON Rust exchanges with Node, as TypeScript types. Their `JsType` impls
//! always compile; the host functions' declarations compile only in tests,
//! where tokamak's `typescript` module writes the declarations its TypeScript
//! is checked against.

use convert_case::{Case, Casing};
use proc_macro::TokenStream;
use proc_macro2::TokenStream as Tokens;
use quote::{format_ident, quote};
use syn::ext::IdentExt;
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::visit_mut::VisitMut;
use syn::{
    Attribute, Data, DataEnum, DeriveInput, Expr, Fields, FieldsNamed, FnArg, GenericArgument,
    ItemFn, LitStr, Pat, Path, PathArguments, ReturnType, Signature, Token, Type, TypeParamBound,
    Visibility,
};

/// Implements `JsType` for a type that rquickjs's `FromJs` and `IntoJs`
/// derives, or serde through `Serde`, convert: for a struct with named fields,
/// an interface of its fields, with the property names those derives use; for
/// a serde enum, a union of its unit variants' names, with
/// `#[serde(tag = ...)]` of objects whose tag names their variant, or with
/// `#[serde(untagged)]` of its variants' contents. A type serde converts
/// through `try_from` or `from` has the type it converts from.
#[proc_macro_derive(JsType, attributes(qjs))]
pub fn derive_js_type(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as DeriveInput);
    js_type(&input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

fn js_type(input: &DeriveInput) -> syn::Result<Tokens> {
    if input.generics.type_params().next().is_some() {
        let message = "JsType is derived for types without type parameters";
        return Err(syn::Error::new_spanned(&input.generics, message));
    }
    let name = &input.ident;
    let typescript = typescript(input)?;
    let (impl_generics, type_generics, where_clause) = input.generics.split_for_impl();
    Ok(quote! {
        impl #impl_generics ::tokamak::typescript::JsType for #name #type_generics #where_clause {
            fn typescript(types: &mut ::tokamak::typescript::Types) -> String {
                #typescript
            }
        }
    })
}

/// The body of a derived `typescript`: the type a serde `try_from` or `from`
/// type converts from, or else a declaration named for the type, or for its
/// `rename`.
fn typescript(input: &DeriveInput) -> syn::Result<Tokens> {
    for option in ["try_from", "from"] {
        if let Some(source) = attribute_value(&input.attrs, "serde", option)? {
            let source: Type = syn::parse_str(&source)?;
            return Ok(quote!(<#source as ::tokamak::typescript::JsType>::typescript(types)));
        }
    }
    let label = match attribute_value(&input.attrs, "serde", "rename")? {
        Some(label) => label,
        None => input.ident.to_string(),
    };
    let rename = rename_all(&input.attrs)?;
    let declaration = match &input.data {
        Data::Struct(data) => {
            let Fields::Named(fields) = &data.fields else {
                return Err(syn::Error::new_spanned(
                    &data.fields,
                    "JsType is derived for structs with named fields",
                ));
            };
            let properties = properties(fields, rename)?;
            quote!(::tokamak::typescript::interface(#label, &[#(#properties),*]))
        }
        Data::Enum(data) => {
            let variants = variants(input, data, rename)?;
            quote!(::tokamak::typescript::alias(#label, &[#(#variants),*].join(" | ")))
        }
        Data::Union(_) => {
            let message = "JsType is derived for structs and enums";
            return Err(syn::Error::new_spanned(input, message));
        }
    };
    Ok(quote!(types.named(#label, ::std::any::type_name::<Self>(), |types| #declaration)))
}

/// Each field's property: its name, whether it may be missing, and its type.
/// A field serde defaults or skips when serializing may be missing, and one
/// serde skips has no property.
fn properties(fields: &FieldsNamed, rename: Option<Rename>) -> syn::Result<Vec<Tokens>> {
    let mut properties = Vec::new();
    for field in &fields.named {
        if attribute_option(&field.attrs, "serde", "skip")?.is_some() {
            continue;
        }
        properties.push(property(field, rename)?);
    }
    Ok(properties)
}

fn property(field: &syn::Field, rename: Option<Rename>) -> syn::Result<Tokens> {
    let ident = field.ident.as_ref();
    let ident = ident.ok_or_else(|| syn::Error::new_spanned(field, "unnamed field"))?;
    let name = js_name(&field.attrs, &ident.unraw().to_string(), rename)?;
    let defaulted = attribute_option(&field.attrs, "serde", "default")?.is_some()
        || attribute_option(&field.attrs, "serde", "skip_serializing_if")?.is_some();
    let field_type = &field.ty;
    Ok(quote! {
        (
            #name,
            #defaulted || <#field_type as ::tokamak::typescript::JsType>::OPTIONAL,
            <#field_type as ::tokamak::typescript::JsType>::typescript(types),
        )
    })
}

/// Each variant's type in a serde enum: its name, or, with a tag, an object
/// of its fields, which the variant's `rename_all` renames, whose tag
/// property is its name. An untagged variant is its content alone.
fn variants(
    input: &DeriveInput,
    data: &DataEnum,
    rename: Option<Rename>,
) -> syn::Result<Vec<Tokens>> {
    if attribute_option(&input.attrs, "serde", "untagged")?.is_some() {
        return data.variants.iter().map(untagged_variant).collect();
    }
    let tag = attribute_value(&input.attrs, "serde", "tag")?;
    data.variants
        .iter()
        .map(|variant| {
            let name = js_name(&variant.attrs, &variant.ident.to_string(), rename)?;
            let literal = format!("{name:?}");
            match (&tag, &variant.fields) {
                (None, Fields::Unit) => Ok(quote!(#literal.to_owned())),
                (Some(tag), Fields::Unit) => {
                    Ok(quote!(::tokamak::typescript::object(&[(#tag, false, #literal.to_owned())])))
                }
                (Some(tag), Fields::Named(fields)) => {
                    let properties = properties(fields, rename_all(&variant.attrs)?)?;
                    Ok(quote! {
                        ::tokamak::typescript::object(&[(#tag, false, #literal.to_owned()), #(#properties),*])
                    })
                }
                _ => Err(syn::Error::new_spanned(
                    variant,
                    "JsType is derived for enums of unit variants, or tagged enums of unit and struct variants",
                )),
            }
        })
        .collect()
}

/// The type of a variant of an untagged serde enum: the type of a newtype
/// variant's field, or an object of a struct variant's fields.
fn untagged_variant(variant: &syn::Variant) -> syn::Result<Tokens> {
    match &variant.fields {
        Fields::Unnamed(fields) if fields.unnamed.len() == 1 => {
            let field_type = &fields.unnamed[0].ty;
            Ok(quote!(<#field_type as ::tokamak::typescript::JsType>::typescript(types)))
        }
        Fields::Named(fields) => {
            let properties = properties(fields, rename_all(&variant.attrs)?)?;
            Ok(quote!(::tokamak::typescript::object(&[#(#properties),*])))
        }
        _ => Err(syn::Error::new_spanned(
            variant,
            "JsType is derived for untagged enums of newtype and struct variants",
        )),
    }
}

/// How a `rename_all` option renames fields and variants.
#[derive(Clone, Copy)]
enum Rename {
    /// rquickjs converts every case, and serde every case but `lowercase`,
    /// as `convert_case` does.
    Case(Case<'static>),
    /// serde's `lowercase` lowercases a whole name.
    Lowercase,
}

impl Rename {
    fn apply(self, name: &str) -> String {
        match self {
            Self::Case(case) => name.to_case(case),
            Self::Lowercase => name.to_lowercase(),
        }
    }
}

/// The JavaScript name of a field, variant or method: its `rename` option,
/// else its Rust name as `rename` renames it.
fn js_name(attributes: &[Attribute], ident: &str, rename: Option<Rename>) -> syn::Result<String> {
    if let Some(name) = attribute_value(attributes, "qjs", "rename")? {
        return Ok(name);
    }
    if let Some(name) = attribute_value(attributes, "serde", "rename")? {
        return Ok(name);
    }
    Ok(rename.map_or_else(|| ident.to_owned(), |rename| rename.apply(ident)))
}

/// How a type's `#[qjs(rename_all = ...)]` or `#[serde(rename_all = ...)]`
/// renames its fields or variants.
fn rename_all(attributes: &[Attribute]) -> syn::Result<Option<Rename>> {
    if let Some(case) = attribute_value(attributes, "qjs", "rename_all")? {
        return named_case(&case, &attributes[0]).map(|case| Some(Rename::Case(case)));
    }
    match attribute_value(attributes, "serde", "rename_all")? {
        None => Ok(None),
        Some(case) if case == "lowercase" => Ok(Some(Rename::Lowercase)),
        Some(case) => named_case(&case, &attributes[0]).map(|case| Some(Rename::Case(case))),
    }
}

/// The case an rquickjs `rename_all` option names.
fn named_case(case: &str, attribute: &Attribute) -> syn::Result<Case<'static>> {
    match case {
        "lowercase" => Ok(Case::Lower),
        "UPPERCASE" => Ok(Case::Upper),
        "camelCase" => Ok(Case::Camel),
        "PascalCase" => Ok(Case::Pascal),
        "snake_case" => Ok(Case::Snake),
        "SCREAMING_SNAKE" => Ok(Case::Constant),
        _ => Err(syn::Error::new_spanned(
            attribute,
            "unknown rename_all case",
        )),
    }
}

/// An option of an attribute: a flag such as `skip`, or a string value such
/// as `rename = "name"`.
#[derive(PartialEq, Eq)]
enum AttributeOption {
    Flag,
    Value(String),
}

impl AttributeOption {
    /// An option given the expression `value`, or a flag.
    fn new(value: Option<Expr>) -> syn::Result<Self> {
        let Some(value) = value else {
            return Ok(Self::Flag);
        };
        Ok(Self::Value(syn::parse2::<LitStr>(quote!(#value))?.value()))
    }
}

/// The string value of the `#[path(...)]` attributes' option `option`, if
/// given.
fn attribute_value(
    attributes: &[Attribute],
    path: &str,
    option: &str,
) -> syn::Result<Option<String>> {
    Ok(match attribute_option(attributes, path, option)? {
        Some(AttributeOption::Value(value)) => Some(value),
        Some(AttributeOption::Flag) | None => None,
    })
}

/// The `#[path(...)]` attributes' option `option`, if given.
fn attribute_option(
    attributes: &[Attribute],
    path: &str,
    option: &str,
) -> syn::Result<Option<AttributeOption>> {
    let mut found = None;
    for attribute in attributes
        .iter()
        .filter(|attribute| attribute.path().is_ident(path))
    {
        attribute.parse_nested_meta(|meta| {
            let given: Option<Expr> = if meta.input.peek(Token![=]) {
                Some(meta.value()?.parse()?)
            } else {
                None
            };
            if meta.path.is_ident(option) {
                found = Some(AttributeOption::new(given)?);
            }
            Ok(())
        })?;
    }
    Ok(found)
}

/// Declares a host function's TypeScript signature, from its parameters'
/// names and types and its result type, as `__host_<name>` beside it.
#[proc_macro_attribute]
pub fn host_function(attribute: TokenStream, item: TokenStream) -> TokenStream {
    let function = syn::parse_macro_input!(item as ItemFn);
    let declaration = if attribute.is_empty() {
        host_declaration(&function).unwrap_or_else(syn::Error::into_compile_error)
    } else {
        syn::Error::new_spanned(&function.sig, "host_function takes no arguments")
            .into_compile_error()
    };
    quote!(#function #declaration).into()
}

fn host_declaration(function: &ItemFn) -> syn::Result<Tokens> {
    let signature = &function.sig;
    if signature.generics.type_params().next().is_some() {
        let message = "host functions take no type parameters";
        return Err(syn::Error::new_spanned(&signature.generics, message));
    }
    let name = format_ident!("__host_{}", signature.ident);
    let visibility = &function.vis;
    let declaration = signature_declaration(signature);
    Ok(quote! {
        #[cfg(test)]
        #[doc(hidden)]
        #visibility fn #name(types: &mut ::tokamak::typescript::Types) -> ::tokamak::typescript::Signature {
            #declaration
        }
    })
}

/// The expression of a function's `::tokamak::typescript::Signature`, which uses
/// `types`. A method's receiver is not a parameter.
fn signature_declaration(signature: &Signature) -> Tokens {
    let parameters = signature
        .inputs
        .iter()
        .enumerate()
        .filter_map(|(index, input)| match input {
            FnArg::Typed(typed) => Some((index, typed)),
            FnArg::Receiver(_) => None,
        })
        .map(|(index, typed)| {
            let name = parameter_name(&typed.pat).unwrap_or_else(|| format!("arg{index}"));
            let parameter_type = erased(&typed.ty);
            quote!(<#parameter_type as ::tokamak::typescript::JsParameter>::parameter(#name, types))
        });
    let returns = returns(signature);
    quote! {
        ::tokamak::typescript::Signature::new(
            ::tokamak::typescript::parameters([#(#parameters),*]),
            #returns,
        )
    }
}

/// A parameter's name in camel case, from a plain or single-field pattern
/// such as `Opt(value)`.
fn parameter_name(pattern: &Pat) -> Option<String> {
    match pattern {
        Pat::Ident(ident) => {
            let name = ident.ident.to_string();
            let name = name.trim_start_matches('_');
            (!name.is_empty()).then(|| name.to_case(Case::Camel))
        }
        Pat::TupleStruct(tuple) if tuple.elems.len() == 1 => parameter_name(&tuple.elems[0]),
        _ => None,
    }
}

/// The TypeScript type of a function's result: a promise of the output for an
/// `async fn` or a function returning `impl Future`.
fn returns(signature: &Signature) -> Tokens {
    let output = match &signature.output {
        ReturnType::Default => quote!(()),
        ReturnType::Type(_, output) => match future_output(output) {
            Some(future) => return promise(&erased(future)),
            None => erased(output),
        },
    };
    if signature.asyncness.is_some() {
        promise(&output)
    } else {
        quote!(<#output as ::tokamak::typescript::JsReturn>::returns(types))
    }
}

fn promise(output: &Tokens) -> Tokens {
    quote!(::tokamak::typescript::promise(&<#output as ::tokamak::typescript::JsReturn>::returns(types)))
}

/// `Output` of `impl Future<Output = Output>`.
fn future_output(output: &Type) -> Option<&Type> {
    let Type::ImplTrait(implementation) = output else {
        return None;
    };
    implementation.bounds.iter().find_map(|bound| {
        let TypeParamBound::Trait(bound) = bound else {
            return None;
        };
        let segment = bound.path.segments.last()?;
        let PathArguments::AngleBracketed(arguments) = &segment.arguments else {
            return None;
        };
        arguments.args.iter().find_map(|argument| match argument {
            GenericArgument::AssocType(output) if output.ident == "Output" => Some(&output.ty),
            _ => None,
        })
    })
}

/// `original` with each named lifetime but `'static` elided, for naming the
/// type outside the function that declares the lifetime.
fn erased(original: &Type) -> Tokens {
    struct Elide;
    impl VisitMut for Elide {
        fn visit_lifetime_mut(&mut self, lifetime: &mut syn::Lifetime) {
            if lifetime.ident != "static" {
                lifetime.ident = format_ident!("_");
            }
        }
    }
    let mut erased = original.clone();
    Elide.visit_type_mut(&mut erased);
    quote!(#erased)
}

/// Implements `JsType` for `Class<Self>` of an rquickjs class JavaScript only
/// holds and hands back to Rust: an interface TypeScript cannot confuse with
/// another class or build itself.
#[proc_macro_derive(JsClass)]
pub fn derive_js_class(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as DeriveInput);
    let class = &input.ident;
    let label = class.to_string();
    let class_type = erased(&syn::parse_quote!(#class));
    quote! {
        impl ::tokamak::typescript::JsType for rquickjs::Class<'_, #class_type> {
            fn typescript(types: &mut ::tokamak::typescript::Types) -> String {
                types.class(#label, ::std::any::type_name::<Self>(), |_| Vec::new())
            }
        }
    }
    .into()
}

/// Implements `JsType` for `Class<Self>` of an rquickjs class from its
/// `#[rquickjs::methods]` impl block, which this attribute must precede: an
/// interface of the methods that block defines.
#[proc_macro_attribute]
pub fn js_methods(attribute: TokenStream, item: TokenStream) -> TokenStream {
    let implementation = syn::parse_macro_input!(item as syn::ItemImpl);
    let declaration = if attribute.is_empty() {
        class_declaration(&implementation).unwrap_or_else(syn::Error::into_compile_error)
    } else {
        syn::Error::new_spanned(&implementation.self_ty, "js_methods takes no arguments")
            .into_compile_error()
    };
    quote!(#implementation #declaration).into()
}

fn class_declaration(implementation: &syn::ItemImpl) -> syn::Result<Tokens> {
    let Type::Path(class) = &*implementation.self_ty else {
        let message = "js_methods implements a named class";
        return Err(syn::Error::new_spanned(&implementation.self_ty, message));
    };
    let label = class
        .path
        .segments
        .last()
        .map(|segment| segment.ident.to_string());
    let label = label.ok_or_else(|| syn::Error::new_spanned(class, "unnamed class"))?;
    let class_type = erased(&implementation.self_ty);
    let case = methods_rename_all(&implementation.attrs)?;
    let mut members = Vec::new();
    for item in &implementation.items {
        let syn::ImplItem::Fn(method) = item else {
            continue;
        };
        if qjs_flag(&method.attrs, "skip")? {
            continue;
        }
        for unsupported in ["set", "prop", "static", "constructor"] {
            if qjs_flag(&method.attrs, unsupported)? {
                let message = format!("js_methods does not describe #[qjs({unsupported})] yet");
                return Err(syn::Error::new_spanned(&method.sig, message));
            }
        }
        let name = js_name(&method.attrs, &method.sig.ident.to_string(), case)?;
        let mut signature = method.sig.clone();
        ReplaceSelf(&implementation.self_ty).visit_signature_mut(&mut signature);
        members.push(if qjs_flag(&method.attrs, "get")? {
            let returns = returns(&signature);
            quote!(::tokamak::typescript::getter(#name, &#returns))
        } else {
            let declaration = signature_declaration(&signature);
            quote!(#declaration.method(#name))
        });
    }
    Ok(quote! {
        impl ::tokamak::typescript::JsType for rquickjs::Class<'_, #class_type> {
            fn typescript(types: &mut ::tokamak::typescript::Types) -> String {
                types.class(#label, ::std::any::type_name::<Self>(), |types| vec![#(#members),*])
            }
        }
    })
}

/// The case `#[rquickjs::methods(rename_all = ...)]` names methods in.
fn methods_rename_all(attributes: &[Attribute]) -> syn::Result<Option<Rename>> {
    let methods = attributes.iter().find(|attribute| {
        attribute
            .path()
            .segments
            .last()
            .is_some_and(|segment| segment.ident == "methods")
    });
    let Some(methods) = methods else {
        return Ok(None);
    };
    let mut case = None;
    if matches!(methods.meta, syn::Meta::List(_)) {
        methods.parse_nested_meta(|meta| {
            let given: LitStr = meta.value()?.parse()?;
            if meta.path.is_ident("rename_all") {
                case = Some(given.value());
            }
            Ok(())
        })?;
    }
    case.map(|case| named_case(&case, methods).map(Rename::Case))
        .transpose()
}

/// Replaces `Self` with the type it names, for naming types outside the impl.
struct ReplaceSelf<'a>(&'a Type);

impl VisitMut for ReplaceSelf<'_> {
    fn visit_type_mut(&mut self, node: &mut Type) {
        if let Type::Path(path) = node
            && path.qself.is_none()
            && path.path.is_ident("Self")
        {
            *node = self.0.clone();
            return;
        }
        syn::visit_mut::visit_type_mut(self, node);
    }
}

/// Whether the `#[qjs(...)]` options include the flag `option`.
fn qjs_flag(attributes: &[Attribute], option: &str) -> syn::Result<bool> {
    Ok(attribute_option(attributes, "qjs", option)? == Some(AttributeOption::Flag))
}

/// Defines `HOST_EXPORTS`, the names a host module declares,
/// `export_host_functions`, which exports each name's native function, and, in
/// tests, `host_declarations`, each name's TypeScript signature. Each function
/// is a path to a function marked `#[host_function]`, in `Async(...)` when it
/// is asynchronous.
#[proc_macro]
pub fn host_functions(input: TokenStream) -> TokenStream {
    let HostFunctions {
        visibility,
        functions,
    } = syn::parse_macro_input!(input as HostFunctions);
    let names = functions.iter().map(|function| &function.name);
    let exported = functions.iter().map(|function| {
        let (name, path) = (&function.name, &function.path);
        if function.asynchronous {
            quote!(exports.export(#name, rquickjs::Function::new(ctx.clone(), rquickjs::function::Async(#path))?)?;)
        } else {
            quote!(exports.export(#name, rquickjs::Function::new(ctx.clone(), #path)?)?;)
        }
    });
    let declarations = functions.iter().map(|function| {
        let name = &function.name;
        let mut path = function.path.clone();
        if let Some(last) = path.segments.last_mut() {
            last.ident = format_ident!("__host_{}", last.ident);
        }
        quote!((#name, #path(types)))
    });
    quote! {
        #visibility const HOST_EXPORTS: &[&str] = &[#(#names),*];

        #visibility fn export_host_functions<'js>(
            ctx: &rquickjs::Ctx<'js>,
            exports: &rquickjs::module::Exports<'js>,
        ) -> rquickjs::Result<()> {
            #(#exported)*
            Ok(())
        }

        #[cfg(test)]
        #visibility fn host_declarations(
            types: &mut ::tokamak::typescript::Types,
        ) -> Vec<(&'static str, ::tokamak::typescript::Signature)> {
            vec![#(#declarations),*]
        }
    }
    .into()
}

struct HostFunctions {
    visibility: Visibility,
    functions: Punctuated<HostFunction, Token![,]>,
}

struct HostFunction {
    name: LitStr,
    path: Path,
    asynchronous: bool,
}

impl Parse for HostFunctions {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let visibility = input.parse()?;
        input.parse::<Token![,]>()?;
        let functions = Punctuated::parse_terminated(input)?;
        Ok(Self {
            visibility,
            functions,
        })
    }
}

impl Parse for HostFunction {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let name = input.parse()?;
        input.parse::<Token![=>]>()?;
        let function: Expr = input.parse()?;
        let (path, asynchronous) = match &function {
            Expr::Path(path) => (path.path.clone(), false),
            Expr::Call(call) => match (&*call.func, call.args.first(), call.args.len()) {
                (Expr::Path(wrapper), Some(Expr::Path(path)), 1)
                    if wrapper.path.is_ident("Async") =>
                {
                    (path.path.clone(), true)
                }
                _ => return Err(not_a_host_function(&function)),
            },
            _ => return Err(not_a_host_function(&function)),
        };
        Ok(Self {
            name,
            path,
            asynchronous,
        })
    }
}

fn not_a_host_function(function: &Expr) -> syn::Error {
    let message = "a host function is a path to a #[host_function] function, in Async(...) when it is asynchronous";
    syn::Error::new_spanned(function, message)
}
