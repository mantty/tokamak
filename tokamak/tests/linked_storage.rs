//! An app reaches the storage part through the entry point its executable
//! exports, as this test executable does.
#![cfg(feature = "native")]

use std::collections::BTreeMap;
use std::fs;
use std::time::Duration;

use tokamak::{
    Config, ModuleType, PackageLayout, Runtime, StorageBinding, WorkerEnvironment, WorkerManifest,
    write_worker, write_worker_environment,
};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

const WORKER: &[u8] = br#"
import { WorkerEntrypoint } from "cloudflare:workers";

export class TokamakEvents extends WorkerEntrypoint {
  async dispatch(name) {
    if (name === "put") await this.env.SESSION.put("visits", "1");
    return { reply: await this.env.SESSION.get("visits"), listened: ["put", "get"] };
  }
}

export default { fetch: () => new Response(null, { status: 404 }) };
"#;

#[test]
fn serves_storage_bindings_through_the_exported_entry_point() -> TestResult {
    let temporary = tempfile::tempdir()?;
    let app = PackageLayout::new(temporary.path().join("app"));
    fs::create_dir_all(app.root())?;
    write_worker_environment(
        &app,
        &WorkerEnvironment {
            vars: BTreeMap::new(),
            storage: vec![StorageBinding::Kv {
                name: "SESSION".to_owned(),
                id: "session".to_owned(),
            }],
        },
    )?;
    fs::write(temporary.path().join("worker.mjs"), WORKER)?;
    write_worker(
        &app,
        temporary.path(),
        &WorkerManifest {
            entry: "worker.mjs".to_owned(),
            modules: BTreeMap::from([("worker.mjs".to_owned(), ModuleType::EsModule)]),
        },
    )?;
    let runtime = Runtime::start(
        Config {
            app,
            state_dir: temporary.path().join("state"),
            storage_dir: temporary.path().join("storage"),
            host: "app.tokamak.local".to_owned(),
            foreground: true,
            plugins: None,
        },
        |_| {},
    )?;

    runtime.emit("put", "{}", Duration::from_secs(5))?;

    assert_eq!(runtime.emit("get", "{}", Duration::from_secs(5))?, r#""1""#);
    Ok(())
}
