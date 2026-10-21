//! Platform bindings: the TypeScript classes and native plugins `tok bindings`
//! generates from the platform SDKs' API descriptions.

mod android;
mod apple;
mod java;
mod kotlin;
mod model;
mod swift;
mod swift_types;
#[cfg(test)]
mod tests;
mod typescript;

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use model::{Kind, Module, NamedKind, Skipped, Ty, Type};

#[derive(Deserialize)]
struct Manifest {
    id: String,
    #[serde(default)]
    bindings: Families,
}

/// What each platform family binds: Apple SDKs by version, Android's by API
/// level.
#[derive(Debug, Deserialize, Default)]
pub(crate) struct Families {
    pub(crate) apple: Option<Family<String>>,
    pub(crate) android: Option<Family<u32>>,
}

/// The modules a family binds, as the SDK `sdk` declares them: app builds
/// compile the generated source with that SDK or a newer one.
#[derive(Debug, Deserialize)]
pub(crate) struct Family<Sdk> {
    pub(crate) sdk: Sdk,
    pub(crate) modules: Vec<String>,
}

/// The generated files of each family, relative to the package.
pub(crate) const APPLE_SOURCE: &str = "apple/Bindings.swift";
const APPLE_MODULE: &str = "apple/bindings.ts";
pub(crate) const ANDROID_SOURCE: &str = "android/Bindings.kt";
const ANDROID_MODULE: &str = "android/bindings.ts";

/// Generates the bindings that the manifest of the plugin `package` lists.
pub(crate) fn generate(package: &Path) -> Result<()> {
    let manifest = package.join("tokamak-plugin.json");
    let manifest: Manifest = serde_json::from_slice(
        &fs::read(&manifest).with_context(|| format!("read {}", manifest.display()))?,
    )
    .with_context(|| format!("invalid {}", manifest.display()))?;
    let Families { apple, android } = &manifest.bindings;
    if apple.is_none() && android.is_none() {
        bail!("tokamak-plugin.json lists no bindings");
    }
    if let Some(apple) = apple {
        let mut module = apple::read(&manifest.id, apple)?;
        prune(&mut module);
        write(package, APPLE_MODULE, &typescript::emit(&module))?;
        write(package, APPLE_SOURCE, &swift::emit(&module, &manifest.id))?;
        report("Apple", APPLE_MODULE, &module);
    }
    if let Some(android) = android {
        let mut module = android::read(&manifest.id, android)?;
        prune(&mut module);
        write(package, ANDROID_MODULE, &typescript::emit(&module))?;
        write(
            package,
            ANDROID_SOURCE,
            &kotlin::emit(&module, &manifest.id),
        )?;
        report("Android", ANDROID_MODULE, &module);
    }
    Ok(())
}

fn write(package: &Path, path: &str, contents: &str) -> Result<()> {
    let path = package.join(path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, contents).with_context(|| format!("write {}", path.display()))
}

fn report(family: &str, path: &str, module: &Module) {
    let members: usize = module
        .types
        .iter()
        .map(|ty| ty.members.len())
        .sum::<usize>()
        + module.globals.len();
    println!(
        "{family}: bound {} types and {members} members in {path}; left out {} symbols:",
        module.types.len(),
        module.skipped.len()
    );
    for skipped in &module.skipped {
        println!("  {}: {}", skipped.symbol, skipped.reason);
    }
}

/// Leaves out what cannot cross between JavaScript and native code: values
/// of types the module does not bind, structs JavaScript cannot construct
/// where native code takes them, and errors as arguments.
fn prune(module: &mut Module) {
    while let Some((position, reason)) = unrepresentable(&module.types) {
        let ty = module.types.remove(position);
        module.skipped.push(skipped(&ty.name(), reason));
    }
    while let Some(ty) = unconstructible(module) {
        if let Kind::Struct { constructible, .. } = &mut module.types[ty].kind {
            *constructible = false;
        }
    }
    let kinds = kinds(&module.types);
    let mut skipped = Vec::new();
    for ty in &mut module.types {
        prune_type(ty, &kinds, &mut skipped);
    }
    module
        .globals
        .retain(|member| keep(member, &kinds, &mut skipped));
    module.skipped.extend(skipped);
}

/// A type JavaScript cannot hold: nested in a type the module leaves out,
/// or made of types it does not bind.
fn unrepresentable(types: &[Type]) -> Option<(usize, &'static str)> {
    let kinds = kinds(types);
    types.iter().enumerate().find_map(|(position, ty)| {
        let parent = &ty.path[..ty.path.len() - 1];
        let reason = match &ty.kind {
            _ if !parent.is_empty() && !kinds.contains_key(parent) => {
                "its enclosing type is not bound"
            }
            Kind::Struct { fields, .. }
                if !fields.iter().all(|field| encodable(&field.ty, &kinds)) =>
            {
                "a field's type is not bound"
            }
            Kind::Alias(target) if !encodable(target, &kinds) => "its aliased type is not bound",
            _ => return None,
        };
        Some((position, reason))
    })
}

/// A struct marked constructible that has a field JavaScript cannot send.
fn unconstructible(module: &Module) -> Option<usize> {
    let kinds = kinds(&module.types);
    module.types.iter().position(|ty| match &ty.kind {
        Kind::Struct {
            fields,
            constructible: true,
        } => !fields.iter().all(|field| decodable(&field.ty, &kinds)),
        _ => false,
    })
}

/// Whether each bound type at a path is a struct JavaScript can construct.
type Kinds = HashMap<Vec<String>, bool>;

fn kinds(types: &[Type]) -> Kinds {
    types
        .iter()
        .map(|ty| {
            let constructible = !matches!(
                ty.kind,
                Kind::Struct {
                    constructible: false,
                    ..
                }
            );
            (ty.path.clone(), constructible)
        })
        .collect()
}

fn prune_type(ty: &mut Type, kinds: &Kinds, skipped: &mut Vec<Skipped>) {
    let name = ty.name();
    if let Kind::Class { superclass, .. } = &mut ty.kind
        && superclass
            .as_ref()
            .is_some_and(|superclass| !kinds.contains_key(superclass))
    {
        *superclass = None;
    }
    if let Kind::Class { callbacks, .. } | Kind::Protocol { callbacks } = &mut ty.kind
        && let Some(methods) = callbacks.as_mut().map(|callbacks| &mut callbacks.methods)
    {
        let required_fails = methods.iter().any(|method| {
            !method.default
                && !method
                    .params
                    .iter()
                    .all(|param| encodable(&param.ty, kinds))
        });
        if required_fails {
            *callbacks = None;
            skipped.push(self::skipped(
                &name,
                "cannot be implemented: a required method's types are not bound",
            ));
        } else {
            methods.retain(|method| {
                let kept = method
                    .params
                    .iter()
                    .all(|param| encodable(&param.ty, kinds));
                if !kept {
                    skipped.push(self::skipped(
                        &format!("{name}.{}", method.name),
                        "a parameter's type is not bound",
                    ));
                }
                kept
            });
        }
    }
    ty.members.retain(|member| keep(member, kinds, skipped));
}

fn keep(member: &model::Member, kinds: &Kinds, skipped: &mut Vec<Skipped>) -> bool {
    let decodable = member
        .params
        .iter()
        .all(|param| self::decodable(&param.ty, kinds));
    let encodable = encodable(&member.result, kinds);
    if !decodable || !encodable {
        let reason = if decodable {
            "its result's type is not bound"
        } else {
            "a parameter's type is not bound"
        };
        skipped.push(self::skipped(&member.key, reason));
    }
    decodable && encodable
}

fn skipped(symbol: &str, reason: &str) -> Skipped {
    Skipped {
        symbol: symbol.to_owned(),
        reason: reason.to_owned(),
    }
}

/// Whether native code can send a value of `ty` to JavaScript.
fn encodable(ty: &Ty, kinds: &Kinds) -> bool {
    match ty {
        Ty::Optional(inner) | Ty::List(inner, _) | Ty::Record(inner) => encodable(inner, kinds),
        Ty::Named(named) => match &named.kind {
            NamedKind::Alias(target) => kinds.contains_key(&named.path) && encodable(target, kinds),
            _ => kinds.contains_key(&named.path),
        },
        _ => true,
    }
}

/// Whether JavaScript can send a value of `ty` to native code.
fn decodable(ty: &Ty, kinds: &Kinds) -> bool {
    match ty {
        Ty::Error => false,
        Ty::Optional(inner) | Ty::List(inner, _) | Ty::Record(inner) => decodable(inner, kinds),
        Ty::Named(named) => match &named.kind {
            NamedKind::Alias(target) => kinds.contains_key(&named.path) && decodable(target, kinds),
            _ => kinds.get(&named.path).copied().unwrap_or(false),
        },
        _ => true,
    }
}
