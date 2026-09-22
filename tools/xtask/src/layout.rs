use std::env;
use std::path::{Path, PathBuf};

use tokamak_cli::{MANIFEST_FILE, Target};

pub(crate) struct WorkspaceLayout {
    root: PathBuf,
}

impl WorkspaceLayout {
    pub(crate) fn from_source() -> Option<Self> {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let root = manifest_dir.parent()?.parent()?.to_path_buf();
        let layout = Self { root };
        (layout.root.join("Cargo.toml").is_file() && layout.root.join("tokamak").is_dir())
            .then_some(layout)
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn platform_pack(&self, target: Target) -> PathBuf {
        self.root
            .join("target/tokamak-platform-packs")
            .join(target.to_string())
    }

    pub(crate) fn recipe_output(&self, target: Target) -> PathBuf {
        self.root
            .join("target/tokamak-platform-pack-staging")
            .join(target.to_string())
    }

    pub(crate) fn platform_recipe(&self, target: Target) -> PathBuf {
        let platform = target.platform();
        self.root
            .join("platforms")
            .join(platform.repository_directory_name())
            .join("build")
            .join(platform.platform_pack_recipe_file_name())
    }

    pub(crate) fn esbuild_host_package(&self, host: &str) -> PathBuf {
        self.root
            .join("tools/esbuild-hosts/node_modules/@esbuild")
            .join(host)
    }

    #[cfg(test)]
    pub(crate) fn from_root(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub(crate) fn manifest(&self, target: Target) -> PathBuf {
        self.platform_pack(target).join(MANIFEST_FILE)
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::WorkspaceLayout;
    use tokamak_cli::Target;

    #[test]
    fn resolves_workspace_artifacts_from_one_root() {
        let layout = WorkspaceLayout {
            root: "/workspace/tokamak".into(),
        };

        assert_eq!(
            layout.platform_pack(Target::MacosArm64),
            Path::new("/workspace/tokamak/target/tokamak-platform-packs/macos-arm64")
        );
        assert_eq!(
            layout.platform_recipe(Target::AndroidArm64),
            Path::new("/workspace/tokamak/platforms/android/build/platform-pack")
        );
        assert_eq!(
            layout.platform_recipe(Target::IosArm64),
            Path::new("/workspace/tokamak/platforms/apple/build/platform-pack")
        );
        assert_eq!(
            layout.platform_recipe(Target::WindowsX64),
            Path::new("/workspace/tokamak/platforms/windows/build/platform-pack.ps1")
        );
        assert_eq!(
            layout.esbuild_host_package("win32-x64"),
            Path::new("/workspace/tokamak/tools/esbuild-hosts/node_modules/@esbuild/win32-x64")
        );
    }
}
