use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use tokamak_cli::{
    PackVariable, PlatformPackManifest, PluginKeyKind, Target, copy_dir_contents, script_command,
    write_manifest,
};

use crate::layout::WorkspaceLayout;
use crate::support::{reset_dir, run};

pub(crate) fn build_source_platform_pack(target: Target) -> Result<PathBuf> {
    let workspace = WorkspaceLayout::from_source()
        .context("source workspace is unavailable; run this command from a tokamak checkout")?;
    build_source_platform_pack_at(&workspace, target)
}

fn build_source_platform_pack_at(workspace: &WorkspaceLayout, target: Target) -> Result<PathBuf> {
    let pack_dir = workspace.platform_pack(target);
    let recipe_output = workspace.recipe_output(target);
    reset_dir(&pack_dir)?;
    reset_dir(&recipe_output)?;

    run_platform_recipe(workspace, target, &recipe_output)?;
    copy_dir_contents(&recipe_output, &pack_dir).context("copy platform-pack artifacts")?;
    fs::remove_dir_all(&recipe_output)?;

    validate_entrypoint(&pack_dir, target)?;

    let manifest = PlatformPackManifest {
        tokamak_version: env!("CARGO_PKG_VERSION").to_owned(),
        target,
        variables: target_variables(workspace, target)?,
        plugin_keys: plugin_keys(workspace, target)?,
    };
    let manifest_path = workspace.manifest(target);
    write_manifest(&manifest_path, &manifest)?;
    Ok(manifest_path)
}

fn run_platform_recipe(workspace: &WorkspaceLayout, target: Target, output: &Path) -> Result<()> {
    let recipe = workspace.platform_recipe(target);
    if !recipe.is_file() {
        bail!("platform-pack recipe is missing: {}", recipe.display());
    }

    run(script_command(&recipe)
        .args(["build", &target.to_string()])
        .arg(output)
        .current_dir(workspace.root()))
    .with_context(|| format!("run platform-pack recipe {}", recipe.display()))
}

/// The variables a pack declares for `target`'s platform namespace.
fn target_variables(
    workspace: &WorkspaceLayout,
    target: Target,
) -> Result<BTreeMap<String, PackVariable>> {
    let path = workspace.platform_build(target).join("variables.json");
    let content = fs::read_to_string(&path)
        .with_context(|| format!("read platform-pack variables {}", path.display()))?;
    let mut namespaces: BTreeMap<String, BTreeMap<String, PackVariable>> =
        serde_json::from_str(&content)
            .with_context(|| format!("parse platform-pack variables {}", path.display()))?;
    let namespace = target.platform().namespace();
    namespaces.remove(namespace).with_context(|| {
        format!(
            "platform-pack variables {} do not declare the {namespace} namespace",
            path.display()
        )
    })
}

/// The keys a pack reads from a plugin's platform section.
fn plugin_keys(
    workspace: &WorkspaceLayout,
    target: Target,
) -> Result<BTreeMap<String, PluginKeyKind>> {
    let path = workspace.platform_build(target).join("plugin-keys.json");
    let content = fs::read_to_string(&path)
        .with_context(|| format!("read platform-pack plugin keys {}", path.display()))?;
    serde_json::from_str(&content)
        .with_context(|| format!("parse platform-pack plugin keys {}", path.display()))
}

fn validate_entrypoint(pack: &Path, target: Target) -> Result<()> {
    let entrypoint = pack.join(target.build_entrypoint_path());
    if entrypoint.is_file() {
        Ok(())
    } else {
        bail!(
            "platform-pack recipe did not produce its entrypoint: {}",
            entrypoint.display()
        )
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use anyhow::Context;

    use super::{plugin_keys, target_variables, validate_entrypoint};
    use tokamak_cli::{PluginKeyKind, Target, VariableKind};

    use crate::layout::WorkspaceLayout;

    #[test]
    fn selects_the_variables_for_the_target_namespace() -> anyhow::Result<()> {
        let workspace = WorkspaceLayout::from_source().context("source workspace")?;
        let simulator = target_variables(&workspace, Target::IosSimulatorArm64)?;
        let macos = target_variables(&workspace, Target::MacosArm64)?;
        let windows = target_variables(&workspace, Target::WindowsX64)?;

        assert_eq!(
            simulator.get("plist").map(|variable| variable.kind),
            Some(VariableKind::Path)
        );
        assert!(simulator.contains_key("provisioning-profile"));
        assert!(!macos.contains_key("provisioning-profile"));
        assert!(windows.is_empty());
        Ok(())
    }

    #[test]
    fn reads_the_plugin_keys_each_pack_declares() -> anyhow::Result<()> {
        let workspace = WorkspaceLayout::from_source().context("source workspace")?;

        assert_eq!(
            plugin_keys(&workspace, Target::IosSimulatorArm64)?.get("sources"),
            Some(&PluginKeyKind::Paths)
        );
        assert!(plugin_keys(&workspace, Target::AndroidArm64)?.contains_key("dependencies"));
        assert!(plugin_keys(&workspace, Target::WindowsX64)?.is_empty());
        Ok(())
    }

    #[test]
    fn rejects_a_pack_without_its_entrypoint() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let result = validate_entrypoint(directory.path(), Target::MacosArm64);

        assert!(result.is_err_and(|error| {
            error
                .to_string()
                .contains("platform-pack recipe did not produce its entrypoint")
        }));
        Ok(())
    }

    #[test]
    fn accepts_the_entrypoint_for_the_target() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        fs::create_dir(directory.path().join("build"))?;
        fs::write(directory.path().join("build/entrypoint.ps1"), "")?;

        validate_entrypoint(directory.path(), Target::WindowsX64)?;
        Ok(())
    }
}
