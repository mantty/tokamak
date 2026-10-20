#![cfg(feature = "native")]

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use serde_json::{Value, json};
use tokamak::{Config, ModuleType, PackageLayout, Runtime, WorkerManifest, write_worker};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
const TIMEOUT: Duration = Duration::from_secs(5);

#[test]
fn main_workers_have_portable_contexts_in_single_and_dual_bundle_apps() -> TestResult {
    for split in [false, true] {
        let directory = tempfile::tempdir()?;
        let app = PackageLayout::new(directory.path().join("app"));
        package(
            &app,
            r"
export default {
  fetch(request, env, ctx) {
    return Response.json({
      context: typeof ctx[Symbol.for('tokamak.native')],
      global: typeof globalThis[Symbol.for('tokamak.native')]
    });
  }
};",
            &json!({}),
        )?;
        if split {
            package(&app.tokamak_worker(), "export default {};", &json!({}))?;
        }
        let runtime = Runtime::start(config(directory.path(), app), |_| {})?;
        let result: Value = serde_json::from_slice(&runtime.fetch("/", TIMEOUT)?)?;
        assert_eq!(
            result,
            json!({"context": "undefined", "global": "undefined"})
        );
    }
    Ok(())
}

#[test]
fn native_http_routes_keep_the_page_path_and_receiving_context() -> TestResult {
    let directory = tempfile::tempdir()?;
    let app = PackageLayout::new(directory.path().join("app"));
    package(
        &app,
        "export default { fetch: () => new Response('main') };",
        &json!({}),
    )?;
    package(
        &app.tokamak_worker(),
        r"
import { WorkerEntrypoint } from 'cloudflare:workers';
export default class extends WorkerEntrypoint {
  async fetch(request) {
    const stage = JSON.parse(new TextDecoder().decode(await this.ctx[Symbol.for('tokamak.native')].call('tokamak', 'lifecycleStage', new Uint8Array())));
    return Response.json({ url: request.url, stage });
  }
}
export class TokamakEvents extends WorkerEntrypoint {
  dispatch() { return { reply: null, listened: [] }; }
}
",
        &json!({}),
    )?;
    let runtime = Runtime::start(config(directory.path(), app), |_| {})?;
    assert_eq!(runtime.fetch("/", TIMEOUT)?, b"main");
    assert_eq!(runtime.fetch("/_tokamak-other", TIMEOUT)?, b"main");
    let result: Value = serde_json::from_slice(&runtime.fetch("/_tokamak/app?value=1", TIMEOUT)?)?;
    assert_eq!(
        result,
        json!({ "url": "https://services.tokamak.local/_tokamak/app?value=1", "stage": "foreground" })
    );
    Ok(())
}

fn package(app: &PackageLayout, source: &str, environment: &Value) -> TestResult {
    package_modules(app, &BTreeMap::from([("worker.mjs", source)]), environment)
}

fn package_modules(
    app: &PackageLayout,
    modules: &BTreeMap<&str, &str>,
    environment: &Value,
) -> TestResult {
    fs::create_dir_all(app.root())?;
    let sources = tempfile::tempdir()?;
    for (name, source) in modules {
        fs::write(sources.path().join(name), source)?;
    }
    write_worker(
        app,
        sources.path(),
        &WorkerManifest {
            entry: "worker.mjs".into(),
            modules: modules
                .keys()
                .map(|name| ((*name).into(), ModuleType::EsModule))
                .collect(),
        },
    )?;
    fs::write(app.worker_environment(), serde_json::to_vec(environment)?)?;
    Ok(())
}

#[test]
fn frontend_rpc_uses_the_packaged_native_worker_and_owns_its_streams() -> TestResult {
    let directory = tempfile::tempdir()?;
    let app = PackageLayout::new(directory.path().join("app"));
    package(
        &app,
        "export default { fetch: () => new Response('main') };",
        &json!({}),
    )?;
    package_modules(
        &app.tokamak_worker(),
        &BTreeMap::from([
            ("worker.mjs", include_str!("fixtures/rpc-worker.mjs")),
            (
                "endpoint.mjs",
                include_str!("../../plugins/core/src/rpc/endpoint.mjs"),
            ),
            (
                "channel.mjs",
                include_str!("../../plugins/core/src/rpc/channel.mjs"),
            ),
            (
                "frames.mjs",
                include_str!("../../plugins/core/src/rpc/frames.mjs"),
            ),
            (
                "streams.mjs",
                include_str!("../../plugins/core/src/rpc/streams.mjs"),
            ),
            (
                "values.mjs",
                include_str!("../../plugins/core/src/rpc/values.mjs"),
            ),
        ]),
        &json!({}),
    )?;
    let runtime = Runtime::start(config(directory.path(), app), |_| {})?;
    let output = Command::new("node")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rpc-client.mjs"))
        .arg(runtime.port().to_string())
        .arg(directory.path().join("state"))
        .output()?;
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

fn config(root: &Path, app: PackageLayout) -> Config {
    Config {
        app,
        state_dir: root.join("state"),
        storage_dir: root.join("storage"),
        host: "services.tokamak.local".into(),
        foreground: true,
        plugins: None,
    }
}

#[test]
fn packaged_workers_call_each_other_and_route_events_to_the_native_worker() -> TestResult {
    let directory = tempfile::tempdir()?;
    let app = PackageLayout::new(directory.path().join("app"));
    package(
        &app,
        r"
import { WorkerEntrypoint } from 'cloudflare:workers';
import { writeFileSync, existsSync } from 'node:fs';
export default class extends WorkerEntrypoint {
  async fetch() {
    writeFileSync('/tmp/owner', 'main');
    const result = await this.env.DEVICE.describe();
    return Response.json({ result, native: typeof this.ctx[Symbol.for('tokamak.native')], file: existsSync('/tmp/owner') });
  }
  name() { return this.env.NAME; }
}
",
        &json!({ "vars": { "NAME": "main" }, "services": { "DEVICE": { "service": "tokamak", "entrypoint": "Device" } } }),
    )?;
    package(
        &app.tokamak_worker(),
        r"
import { WorkerEntrypoint } from 'cloudflare:workers';
import { existsSync } from 'node:fs';
export class Device extends WorkerEntrypoint {
  async describe() {
    return {
      name: this.env.NAME, main: await this.env.MAIN.name(), file: existsSync('/tmp/owner'),
      stage: JSON.parse(new TextDecoder().decode(await this.ctx[Symbol.for('tokamak.native')].call('tokamak', 'lifecycleStage', new Uint8Array()))),
    };
  }
}
export class TokamakEvents extends WorkerEntrypoint {
  async dispatch(name) {
    return { reply: { name, receiver: this.env.NAME, main: await this.env.MAIN.name() }, listened: ['echo'] };
  }
}
export default { fetch: () => new Response('device') };
",
        &json!({ "vars": { "NAME": "device" }, "services": { "MAIN": { "service": "main", "entrypoint": "default" } } }),
    )?;
    let runtime = Runtime::start(config(directory.path(), app), |_| {})?;

    let response: Value = serde_json::from_slice(&runtime.fetch("/", TIMEOUT)?)?;
    assert_eq!(
        response,
        json!({
            "result": { "name": "device", "main": "main", "file": false, "stage": "foreground" },
            "native": "undefined", "file": true,
        })
    );
    let event: Value = serde_json::from_str(&runtime.emit("echo", "{}", TIMEOUT)?)?;
    assert_eq!(
        event,
        json!({ "name": "echo", "receiver": "device", "main": "main" })
    );
    Ok(())
}

#[test]
fn invalid_packaged_service_targets_and_binding_collisions_fail_at_startup() -> TestResult {
    for environment in [
        json!({ "services": { "DEVICE": { "service": "missing", "entrypoint": "default" } } }),
        json!({ "vars": { "DEVICE": 1 }, "services": { "DEVICE": { "service": "main", "entrypoint": "default" } } }),
        json!({ "storage": [{ "type": "kv", "name": "DEVICE", "id": "device" }], "services": { "DEVICE": { "service": "main", "entrypoint": "default" } } }),
        json!({ "services": { "DEVICE": { "service": "main", "entrypoint": "" } } }),
    ] {
        let directory = tempfile::tempdir()?;
        let app = PackageLayout::new(directory.path().join("app"));
        package(
            &app,
            "export default { fetch: () => new Response('ok') };",
            &environment,
        )?;
        let error = Runtime::start(config(directory.path(), app), |_| {})
            .err()
            .ok_or("invalid service graph started")?;
        assert!(
            error.to_string().contains("Service binding DEVICE"),
            "{error}"
        );
    }
    Ok(())
}

#[test]
fn workers_share_named_stores_and_preserve_each_others_storage_after_restart() -> TestResult {
    let directory = tempfile::tempdir()?;
    let app = PackageLayout::new(directory.path().join("app"));
    package(
        &app,
        r"
import { WorkerEntrypoint } from 'cloudflare:workers';
export default class extends WorkerEntrypoint {
  async fetch(request) {
    if (new URL(request.url).pathname === '/write') {
      await this.env.FIRST.put('value', 'first');
      await this.env.SHARED.put('value', 'shared');
      await this.env.DEVICE.save();
    }
    return Response.json({ first: await this.env.FIRST.get('value'), device: await this.env.DEVICE.read() });
  }
}
",
        &json!({ "storage": [
        { "type": "kv", "name": "FIRST", "id": "first" },
        { "type": "kv", "name": "SHARED", "id": "shared" },
    ], "services": { "DEVICE": { "service": "tokamak", "entrypoint": "default" } } }),
    )?;
    package(
        &app.tokamak_worker(),
        r"
import { WorkerEntrypoint } from 'cloudflare:workers';
export default class extends WorkerEntrypoint {
  async save() { await this.env.SECOND.put('value', 'second'); }
  async read() { return { second: await this.env.SECOND.get('value'), shared: await this.env.ALIAS.get('value'), firstBinding: typeof this.env.FIRST }; }
}
export class TokamakEvents extends WorkerEntrypoint {
  dispatch() { return { reply: null, listened: [] }; }
}
",
        &json!({ "storage": [
        { "type": "kv", "name": "SECOND", "id": "second" },
        { "type": "kv", "name": "ALIAS", "id": "shared" },
    ] }),
    )?;
    for path in ["/write", "/read"] {
        let runtime = Runtime::start(config(directory.path(), app.clone()), |_| {})?;
        let response: Value = serde_json::from_slice(&runtime.fetch(path, TIMEOUT)?)?;
        assert_eq!(
            response,
            json!({ "first": "first", "device": { "second": "second", "shared": "shared", "firstBinding": "undefined" } })
        );
    }
    Ok(())
}

#[test]
fn a_broken_native_bundle_preserves_existing_stores() -> TestResult {
    let directory = tempfile::tempdir()?;
    let app = PackageLayout::new(directory.path().join("app"));
    package(
        &app,
        "export default { fetch: () => new Response('ok') };",
        &json!({}),
    )?;
    let native = app.tokamak_worker();
    package(&native, "export default {};", &json!({}))?;
    fs::remove_dir_all(native.worker_modules())?;
    let config = config(directory.path(), app);
    let stored = config.storage_dir.join("kv/previous.sqlite");
    fs::create_dir_all(stored.parent().ok_or("missing storage parent")?)?;
    fs::write(&stored, b"previous app data")?;

    let error = Runtime::start(config, |_| {})
        .err()
        .ok_or("broken bundle started")?;

    assert!(
        error.to_string().contains("entry module is missing"),
        "{error}"
    );
    assert_eq!(fs::read(stored)?, b"previous app data");
    Ok(())
}
