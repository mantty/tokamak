use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, atomic::AtomicBool, mpsc};

use crate::dispatcher::execute_request;
use crate::gateway::{Job, JobResponse, Lifecycle};
use crate::quickjs::{RuntimeConfig, WorkerBundle};
use crate::transport::{HttpBody, HttpRequest};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/quickjs_runtime")
}

fn bundle_worker(entry: &Path, output: &Path) -> TestResult<WorkerBundle> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or("no workspace")?;
    let host = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "darwin-arm64/bin/esbuild",
        ("macos", "x86_64") => "darwin-x64/bin/esbuild",
        ("linux", "x86_64") => "linux-x64/bin/esbuild",
        ("windows", "x86_64") => "win32-x64/esbuild.exe",
        _ => return Err("unsupported esbuild test host".into()),
    };
    let esbuild = workspace
        .join("tools/esbuild-hosts/node_modules/@esbuild")
        .join(host);
    let result = Command::new(esbuild)
        .args([
            "--bundle",
            "--splitting",
            "--format=esm",
            "--platform=neutral",
            "--target=es2022",
            "--entry-names=entry",
            "--chunk-names=chunks/[name]-[hash]",
            "--log-level=error",
            "--external:cloudflare:workers",
            "--external:node:events",
            "--external:events",
            "--external:node:stream",
            "--external:stream",
            "--external:node:process",
            "--external:process",
            "--external:node:fs",
            "--external:fs",
            "--external:node:fs/promises",
            "--external:fs/promises",
        ])
        .arg(format!("--outdir={}", output.display()))
        .arg(entry)
        .output()?;
    if !result.status.success() {
        return Err(String::from_utf8_lossy(&result.stderr).into_owned().into());
    }
    for file in walkdir::WalkDir::new(output) {
        let file = file?;
        if !file.file_type().is_file() || file.path().extension().is_none_or(|ext| ext != "js") {
            continue;
        }
        let name = file
            .path()
            .strip_prefix(output)?
            .to_str()
            .ok_or("non-UTF8 module")?
            .replace('\\', "/");
        let bytecode = crate::compile_module(&name, &fs::read(file.path())?)?;
        fs::write(output.join(format!("{name}.qjs")), bytecode)?;
    }
    Ok(WorkerBundle::from_modules("entry.js", output, output))
}

fn request(worker: &WorkerBundle, flag: &str) -> TestResult<Vec<u8>> {
    let directory = tempfile::tempdir()?;
    let config = RuntimeConfig {
        assets: None,
        cache: directory.path().join("cache"),
        environment: BTreeMap::from([("FLAG".to_owned(), serde_json::json!(flag))]),
    };
    let accepting = Arc::new(AtomicBool::new(true));
    let lifecycle = Lifecycle::new();
    let execution = lifecycle.enter(&accepting).ok_or("request rejected")?;
    let (sender, receiver) = mpsc::sync_channel(1);
    execute_request(
        worker,
        &config,
        Job {
            request: HttpRequest {
                method: "GET".to_owned(),
                target: "/".to_owned(),
                url: "https://app.tokamak.local/".to_owned(),
                headers: BTreeMap::new(),
                body: None,
            },
            response: sender,
            websocket: None,
        },
        &execution,
        &accepting,
    )?;
    let JobResponse::Http(response) = receiver.recv()? else {
        return Err("unexpected websocket".into());
    };
    assert_eq!(response.status, 200);
    let HttpBody::Buffered(body) = response.body else {
        return Err("unexpected stream".into());
    };
    Ok(body)
}

#[test]
fn packaged_globals_precede_application_modules() -> TestResult {
    let directory = tempfile::tempdir()?;
    let source = directory.path().join("startup.mjs");
    fs::write(
        &source,
        "const decoder = new TextDecoder(); const value = decoder.decode(new TextEncoder().encode('ready')); export default { async fetch() { return new Response(value); } };",
    )?;
    let worker = bundle_worker(&source, &directory.path().join("modules"))?;
    assert_eq!(request(&worker, "first")?, b"ready");
    Ok(())
}

#[test]
fn bundled_worker_matches_cloudflare_node_compat() -> TestResult {
    let reference = Command::new("node")
        .arg(fixture_root().join("workerd-reference.mjs"))
        .output()?;
    if !reference.status.success() {
        return Err(format!(
            "workerd reference failed: {}",
            String::from_utf8_lossy(&reference.stderr)
        )
        .into());
    }
    let expected: serde_json::Value = serde_json::from_slice(&reference.stdout)?;
    let directory = tempfile::tempdir()?;
    let worker = bundle_worker(&fixture_root().join("startup.mjs"), directory.path())?;
    let actual: serde_json::Value = serde_json::from_slice(&request(&worker, "enabled")?)?;
    for (name, expected) in expected.as_object().ok_or("reference is not an object")? {
        assert_eq!(
            actual.get(name),
            Some(expected),
            "Cloudflare contract: {name}"
        );
    }
    Ok(())
}

#[test]
fn builtin_modules_and_request_state_are_isolated() -> TestResult {
    let directory = tempfile::tempdir()?;
    let source = directory.path().join("isolation.mjs");
    fs::write(
        &source,
        r"
import { env, waitUntil } from 'cloudflare:workers';
import fs from 'node:fs';
import streams from 'node:stream';
import promises from 'node:fs/promises';
import bareFs from 'fs';
import barePromises from 'fs/promises';
import bareStreams from 'stream';
import processModule from 'process';
import events from 'events';
let calls = 0;
export default { async fetch() {
  if (fs.existsSync('/tmp/previous')) throw new Error('tmp leaked');
  const stream = fs.createReadStream('/dev/null');
  if (!(stream instanceof streams.Readable)) throw new Error('stream identity');
  if (fs.promises !== promises) throw new Error('fs promises identity');
  if (bareFs !== fs || barePromises !== promises || bareStreams !== streams || processModule !== process) throw new Error('alias identity');
  for (const [name, value] of [['fs', fs], ['fs/promises', promises], ['events', events], ['stream', streams], ['process', process]]) {
    if ((await import(name)).default !== value || (await import('node:' + name)).default !== value) throw new Error('dynamic alias identity');
  }
  waitUntil(Promise.resolve().then(() => fs.writeFileSync('/tmp/previous', env.FLAG)));
  return new Response(JSON.stringify({ calls: ++calls, flag: env.FLAG }));
} };
",
    )?;
    let worker = bundle_worker(&source, &directory.path().join("modules"))?;
    assert_eq!(request(&worker, "first")?, br#"{"calls":1,"flag":"first"}"#);
    assert_eq!(
        request(&worker, "second")?,
        br#"{"calls":1,"flag":"second"}"#
    );
    Ok(())
}

#[test]
fn imported_wait_until_retains_pending_work() -> TestResult {
    let directory = tempfile::tempdir()?;
    let source = directory.path().join("pending.mjs");
    fs::write(
        &source,
        r"
import { waitUntil } from 'cloudflare:workers';
export default { async fetch() {
  waitUntil(Promise.resolve().then(() => waitUntil(new Promise(() => {}))));
  return new Response('ready');
} };
",
    )?;
    let worker = bundle_worker(&source, &directory.path().join("modules"))?;
    let error = request(&worker, "first")
        .err()
        .ok_or("unsettled waitUntil was discarded")?;
    assert!(error.to_string().contains("waitUntil"), "{error}");
    Ok(())
}

#[test]
fn application_imports_cannot_access_runtime_internals() -> TestResult {
    let directory = tempfile::tempdir()?;
    for name in [
        "tokamak:host",
        "./tokamak:globals/web.mjs",
        "node:unsupported",
    ] {
        let source = format!(
            "export default {{ async fetch() {{ await import('{name}'); return new Response('unexpected'); }} }};"
        );
        let worker = WorkerBundle::from_bytecode(
            crate::compile_worker(source.as_bytes())?,
            directory.path(),
        );
        assert!(
            request(&worker, "first").is_err(),
            "private import succeeded: {name}"
        );
    }
    Ok(())
}
