//! Macros that describe the Rust values tokamak hands to JavaScript as
//! TypeScript types. What they generate for that compiles only in tests, where
//! tokamak's `typescript` module writes the declarations its TypeScript is
//! checked against.

use convert_case::{Case, Casing};
use proc_macro::TokenStream;
use proc_macro2::TokenStream as Tokens;
use quote::{format_ident, quote};
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::visit_mut::VisitMut;
use syn::{
    Attribute, Data, DeriveInput, Expr, Field, Fields, FnArg, GenericArgument, ItemFn, LitStr, Pat,
    Path, PathArguments, ReturnType, Signature, Token, Type, TypeParamBound, Visibility,
};

/// Implements `JsType` for a struct that rquickjs's `FromJs` and `IntoJs`
/// derives convert: an interface of a struct's fields, with the property names
/// those derives use, or a tuple of a tuple struct's fields.
#[proc_macro_derive(JsType, attributes(qjs))]
pub fn derive_js_type(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as DeriveInput);
    js_type(&input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

fn js_type(input: &DeriveInput) -> syn::Result<Tokens> {
    let Data::Struct(data) = &input.data else {
        return Err(syn::Error::new_spanned(
            input,
            "JsType is derived for structs",
        ));
    };
    if input.generics.type_params().next().is_some() {
        let message = "JsType is derived for structs without type parameters";
        return Err(syn::Error::new_spanned(&input.generics, message));
    }
    let name = &input.ident;
    let label = name.to_string();
    let declaration = match &data.fields {
        Fields::Named(fields) => {
            let case = rename_all(&input.attrs)?;
            let properties = fields
                .named
                .iter()
                .map(|field| property(field, case))
                .collect::<syn::Result<Vec<_>>>()?;
            quote!(crate::typescript::interface(#label, &[#(#properties),*]))
        }
        Fields::Unnamed(fields) => {
            let elements = fields.unnamed.iter().map(|field| {
                let field_type = &field.ty;
                quote!(<#field_type as crate::typescript::JsType>::typescript(types))
            });
            quote!(crate::typescript::alias(#label, &crate::typescript::tuple(&[#(#elements),*])))
        }
        Fields::Unit => quote!(crate::typescript::alias(#label, "undefined")),
    };
    let (impl_generics, type_generics, where_clause) = input.generics.split_for_impl();
    Ok(quote! {
        #[cfg(test)]
        impl #impl_generics crate::typescript::JsType for #name #type_generics #where_clause {
            fn typescript(types: &mut crate::typescript::Types) -> String {
                types.named(#label, ::std::any::type_name::<Self>(), |types| #declaration)
            }
        }
    })
}

/// A field's property: its name, whether it may be missing, and its type.
fn property(field: &Field, case: Option<Case<'static>>) -> syn::Result<Tokens> {
    let name = match qjs_value(&field.attrs, "rename")? {
        Some(name) => name,
        None => {
            let ident = field.ident.as_ref().map(ToString::to_string);
            let ident = ident.ok_or_else(|| syn::Error::new_spanned(field, "unnamed field"))?;
            case.map_or_else(|| ident.clone(), |case| ident.to_case(case))
        }
    };
    let field_type = &field.ty;
    Ok(quote! {
        (
            #name,
            <#field_type as crate::typescript::JsType>::OPTIONAL,
            <#field_type as crate::typescript::JsType>::typescript(types),
        )
    })
}

/// The case `#[qjs(rename_all = ...)]` names fields in, as rquickjs maps it.
fn rename_all(attributes: &[Attribute]) -> syn::Result<Option<Case<'static>>> {
    let Some(case) = qjs_value(attributes, "rename_all")? else {
        return Ok(None);
    };
    named_case(&case, &attributes[0]).map(Some)
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

/// The string value of the `#[qjs(...)]` option `option`, if given.
fn qjs_value(attributes: &[Attribute], option: &str) -> syn::Result<Option<String>> {
    let mut value = None;
    for attribute in attributes
        .iter()
        .filter(|attribute| attribute.path().is_ident("qjs"))
    {
        attribute.parse_nested_meta(|meta| {
            if !meta.input.peek(Token![=]) {
                return Ok(());
            }
            let given: Expr = meta.value()?.parse()?;
            if meta.path.is_ident(option) {
                let text: LitStr = syn::parse2(quote!(#given))?;
                value = Some(text.value());
            }
            Ok(())
        })?;
    }
    Ok(value)
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
        #visibility fn #name(types: &mut crate::typescript::Types) -> crate::typescript::Signature {
            #declaration
        }
    })
}

/// The expression of a function's `crate::typescript::Signature`, which uses
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
            quote!(<#parameter_type as crate::typescript::JsParameter>::parameter(#name, types))
        });
    let returns = returns(signature);
    quote! {
        crate::typescript::Signature::new(
            crate::typescript::parameters([#(#parameters),*]),
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
        quote!(<#output as crate::typescript::JsReturn>::returns(types))
    }
}

fn promise(output: &Tokens) -> Tokens {
    quote!(crate::typescript::promise(&<#output as crate::typescript::JsReturn>::returns(types)))
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
        #[cfg(test)]
        impl crate::typescript::JsType for rquickjs::Class<'_, #class_type> {
            fn typescript(types: &mut crate::typescript::Types) -> String {
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
        let name = match qjs_value(&method.attrs, "rename")? {
            Some(name) => name,
            None => {
                let ident = method.sig.ident.to_string();
                case.map_or_else(|| ident.clone(), |case| ident.to_case(case))
            }
        };
        let mut signature = method.sig.clone();
        ReplaceSelf(&implementation.self_ty).visit_signature_mut(&mut signature);
        members.push(if qjs_flag(&method.attrs, "get")? {
            let returns = returns(&signature);
            quote!(crate::typescript::getter(#name, &#returns))
        } else {
            let declaration = signature_declaration(&signature);
            quote!(#declaration.method(#name))
        });
    }
    Ok(quote! {
        #[cfg(test)]
        impl crate::typescript::JsType for rquickjs::Class<'_, #class_type> {
            fn typescript(types: &mut crate::typescript::Types) -> String {
                types.class(#label, ::std::any::type_name::<Self>(), |types| vec![#(#members),*])
            }
        }
    })
}

/// The case `#[rquickjs::methods(rename_all = ...)]` names methods in.
fn methods_rename_all(attributes: &[Attribute]) -> syn::Result<Option<Case<'static>>> {
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
    case.map(|case| named_case(&case, methods)).transpose()
}

/// Replaces `Self` with the type it names, for naming types outside the impl.
struct ReplaceSelf<'a>(&'a Type);

impl VisitMut for ReplaceSelf<'_> {
    fn visit_type_mut(&mut self, node: &mut Type) {
        if let Type::Path(path) = node {
            if path.qself.is_none() && path.path.is_ident("Self") {
                *node = self.0.clone();
                return;
            }
        }
        syn::visit_mut::visit_type_mut(self, node);
    }
}

/// Whether the `#[qjs(...)]` options include the flag `option`.
fn qjs_flag(attributes: &[Attribute], option: &str) -> syn::Result<bool> {
    let mut found = false;
    for attribute in attributes
        .iter()
        .filter(|attribute| attribute.path().is_ident("qjs"))
    {
        attribute.parse_nested_meta(|meta| {
            if meta.input.peek(Token![=]) {
                let _: Expr = meta.value()?.parse()?;
            } else if meta.path.is_ident(option) {
                found = true;
            }
            Ok(())
        })?;
    }
    Ok(found)
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
        match function.asynchronous {
            true => quote!(exports.export(#name, rquickjs::Function::new(ctx.clone(), rquickjs::function::Async(#path))?)?;),
            false => quote!(exports.export(#name, rquickjs::Function::new(ctx.clone(), #path)?)?;),
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
            types: &mut crate::typescript::Types,
        ) -> Vec<(&'static str, crate::typescript::Signature)> {
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
