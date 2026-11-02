//! The TypeScript types of the JSON that tok and the runtime exchange with
//! Node, and the plugin id words tok reserves: the CLI's npm package reads
//! them, and `@tokamakdev/plugin` exports the types for plugins.

use tokamak::native_protocol::Header;
use tokamak::typescript::{JsType, Types};

use super::plugins::{BuildScriptInput, Plugin, RESERVED_WORDS};
use super::vite::{ConfigReport, ServerFile};

/// Each generated module, by its path in the repository.
fn contracts() -> serde_json::Result<[(&'static str, String); 2]> {
    let mut npm = Types::default();
    ConfigReport::typescript(&mut npm);
    ServerFile::typescript(&mut npm);
    Plugin::typescript(&mut npm);
    let mut plugin = Types::default();
    Header::typescript(&mut plugin);
    BuildScriptInput::typescript(&mut plugin);
    let module = |types: &Types| {
        format!(
            "{}\n{}",
            concat!(
                "// Generated from tok's Rust code; do not edit. Regenerate with\n",
                "// TOKAMAK_GENERATE=1 cargo test -p tokamak-cli generated_contracts",
            ),
            types.exports()
        )
    };
    let reserved = format!(
        "\n/** The words a plugin id must not be or start with, since tok's options use them. */\n\
         export const reservedPluginWords: readonly string[] = {};\n",
        serde_json::to_string_pretty(RESERVED_WORDS)?
    );
    Ok([
        (
            "tokamak-cli/npm/src/contracts.generated.mts",
            module(&npm) + &reserved,
        ),
        ("plugins/core/src/contracts.generated.ts", module(&plugin)),
    ])
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    #[test]
    fn generated_contracts_are_current() -> std::io::Result<()> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        for (path, generated) in super::contracts()? {
            if std::env::var_os("TOKAMAK_GENERATE").is_some() {
                std::fs::write(root.join(path), &generated)?;
            }
            let committed = std::fs::read_to_string(root.join(path)).unwrap_or_default();
            assert!(
                committed.replace("\r\n", "\n") == generated,
                "{path} is out of date; regenerate it with \
                 TOKAMAK_GENERATE=1 cargo test -p tokamak-cli generated_contracts"
            );
        }
        Ok(())
    }
}
