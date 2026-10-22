use super::{TestResult, fixture_root, module_worker, request};
use std::fs;
use std::process::Command;

#[test]
fn wasm_worker_matches_cloudflare() -> TestResult {
    let directory = tempfile::tempdir()?;
    let input = directory.path().join("input");
    fs::create_dir(&input)?;
    let fixtures = fixture_root().join("wasm");
    fs::write(
        input.join("core.wasm"),
        wat::parse_file(fixtures.join("core.wat"))?,
    )?;
    fs::copy(fixtures.join("contracts.mjs"), input.join("contracts.mjs"))?;
    let worker = module_worker(&input, "contracts.mjs", &directory.path().join("app"))?;
    let actual: serde_json::Value = serde_json::from_slice(&request(&worker, "")?)?;
    assert_eq!(actual["add"], 42);
    assert_eq!(actual["exceptionIdentity"], true);
    assert_eq!(actual["memoryWrite"], 456);
    let reference = Command::new("node")
        .arg(fixture_root().join("wasm-reference.mjs"))
        .arg(&input)
        .output()?;
    assert!(
        reference.status.success(),
        "{}",
        String::from_utf8_lossy(&reference.stderr)
    );
    let expected: serde_json::Value = serde_json::from_slice(&reference.stdout)?;
    assert_eq!(actual, expected);
    Ok(())
}

#[test]
fn wasm_execution_can_be_cancelled() -> TestResult {
    use std::io::{BufRead, Write};
    use std::process::Stdio;
    use std::time::{Duration, Instant};

    const MODE: &str = "TOKAMAK_WASM_CANCEL_CHILD";
    if let Ok(mode) = std::env::var(MODE) {
        return cancellation_child(mode == "start");
    }
    for mode in ["call", "start"] {
        let mut child = Command::new(std::env::current_exe()?)
            .args([
                "--exact",
                "runtime_contract_tests::wasm::wasm_execution_can_be_cancelled",
                "--nocapture",
            ])
            .env(MODE, mode)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()?;
        let stdout = child.stdout.take().ok_or("missing child stdout")?;
        let (ready, entered) = std::sync::mpsc::channel();
        let reader = std::thread::spawn(move || {
            for line in std::io::BufReader::new(stdout)
                .lines()
                .map_while(Result::ok)
            {
                if line == "WASM_LOOP_ENTERED" {
                    let _ = ready.send(());
                }
            }
        });
        let result = (|| -> TestResult {
            entered.recv_timeout(Duration::from_secs(10))?;
            child
                .stdin
                .take()
                .ok_or("missing child stdin")?
                .write_all(b"x")?;
            let deadline = Instant::now() + Duration::from_secs(3);
            while Instant::now() < deadline {
                if let Some(status) = child.try_wait()? {
                    assert!(
                        status.success(),
                        "{mode} cancellation child failed: {status}"
                    );
                    return Ok(());
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err("Wasm execution did not stop after cancellation".into())
        })();
        let _ = child.kill();
        child.wait()?;
        reader.join().map_err(|_| "output reader panicked")?;
        result?;
    }
    Ok(())
}

fn cancellation_child(start: bool) -> TestResult {
    use std::io::Read;

    let directory = tempfile::tempdir()?;
    let input = directory.path().join("input");
    fs::create_dir(&input)?;
    let start_section = if start { "(start $loop)" } else { "" };
    let wasm = wat::parse_str(format!(
        r#"(module (import "env" "entered" (func $entered))
        (func $loop (export "run") call $entered (loop $forever br $forever)) {start_section})"#
    ))?;
    fs::write(input.join("loop.wasm"), wasm)?;
    fs::write(
        input.join("worker.mjs"),
        "import module from './loop.wasm'; export default { fetch() { const instance = new WebAssembly.Instance(module, {env: {entered() { console.log('WASM_LOOP_ENTERED'); }}}); instance.exports.run(); return new Response('unreachable'); } };",
    )?;
    let worker = module_worker(&input, "worker.mjs", &directory.path().join("app"))?;
    let runtime = tokio::runtime::Runtime::new()?;
    let _guard = runtime.enter();
    let stopped = tokio_util::sync::CancellationToken::new();
    let interrupt = stopped.clone();
    let cancellation = std::thread::spawn(move || -> std::io::Result<()> {
        let mut byte = [0];
        std::io::stdin().read_exact(&mut byte)?;
        stopped.cancel();
        Ok(())
    });
    let (response, _) = flume::bounded(1);
    let job = crate::gateway::Job {
        request: super::http_request("GET", "/", None),
        response,
        websocket: None,
    };
    let error = crate::dispatcher::execute_request(
        &worker,
        &crate::tests::runtime_config(),
        job,
        &interrupt,
    )
    .err()
    .ok_or("Wasm loop completed without interruption")?
    .to_string();
    cancellation
        .join()
        .map_err(|_| "cancellation thread panicked")??;
    assert!(
        error.contains("interrupted")
            || error.contains("stopped")
            || error.contains("Exception generated by QuickJS"),
        "{error}"
    );
    Ok(())
}

#[test]
fn wasm_rejects_invalid_imports() -> TestResult {
    for import in [
        "(import \"env\" \"value\" (memory 1))",
        "(import \"env\" \"value\" (table 1 funcref))",
        "(import \"env\" \"value\" (global (mut i32)))",
    ] {
        let directory = tempfile::tempdir()?;
        let input = directory.path().join("input");
        fs::create_dir(&input)?;
        fs::write(
            input.join("unsupported.wasm"),
            wat::parse_str(format!("(module {import})"))?,
        )?;
        fs::write(
            input.join("worker.mjs"),
            "import module from './unsupported.wasm'; export default { fetch() { try { new WebAssembly.Instance(module, {env: {value: 0}}); return new Response('ignored'); } catch (e) { return new Response(e.name); } } };",
        )?;
        let worker = module_worker(&input, "worker.mjs", &directory.path().join("app"))?;
        assert_eq!(request(&worker, "")?, b"LinkError");
    }
    Ok(())
}

#[test]
fn wasm_shared_imports_tables_references_and_simd_match_cloudflare() -> TestResult {
    let directory = tempfile::tempdir()?;
    let input = directory.path().join("input");
    fs::create_dir(&input)?;
    let fixtures = fixture_root().join("wasm");
    for name in ["imports", "simd", "forward", "failed"] {
        fs::write(
            input.join(format!("{name}.wasm")),
            wat::parse_file(fixtures.join(format!("{name}.wat")))?,
        )?;
    }
    fs::copy(
        fixtures.join("import-contracts.mjs"),
        input.join("contracts.mjs"),
    )?;
    let reference = Command::new("node")
        .arg(fixture_root().join("wasm-reference.mjs"))
        .arg(&input)
        .output()?;
    assert!(
        reference.status.success(),
        "{}",
        String::from_utf8_lossy(&reference.stderr)
    );
    let expected: serde_json::Value = serde_json::from_slice(&reference.stdout)?;
    assert_eq!(expected["simd"], true, "workerd reference: {expected}");
    let worker = module_worker(&input, "contracts.mjs", &directory.path().join("app"))?;
    let actual: serde_json::Value = serde_json::from_slice(&request(&worker, "")?)?;
    assert_eq!(actual, expected);
    Ok(())
}

#[test]
fn wasm_shared_and_resizable_memory_matches_cloudflare() -> TestResult {
    let directory = tempfile::tempdir()?;
    let input = directory.path().join("input");
    fs::create_dir(&input)?;
    let fixtures = fixture_root().join("wasm");
    fs::write(
        input.join("shared.wasm"),
        wat::parse_file(fixtures.join("shared.wat"))?,
    )?;
    fs::copy(
        fixtures.join("memory-contracts.mjs"),
        input.join("contracts.mjs"),
    )?;
    let reference = Command::new("node")
        .arg(fixture_root().join("wasm-reference.mjs"))
        .arg(&input)
        .output()?;
    assert!(
        reference.status.success(),
        "{}",
        String::from_utf8_lossy(&reference.stderr)
    );
    let expected: serde_json::Value = serde_json::from_slice(&reference.stdout)?;
    assert_eq!(expected["resizableMemory"], true, "workerd: {expected}");
    let worker = module_worker(&input, "contracts.mjs", &directory.path().join("app"))?;
    let actual: serde_json::Value = serde_json::from_slice(&request(&worker, "")?)?;
    assert_eq!(actual, expected);
    Ok(())
}

#[test]
fn wasm_relaxed_simd_matches_cloudflare() -> TestResult {
    let directory = tempfile::tempdir()?;
    let input = directory.path().join("input");
    fs::create_dir(&input)?;
    let fixtures = fixture_root().join("wasm");
    fs::write(
        input.join("relaxed.wasm"),
        wat::parse_file(fixtures.join("relaxed.wat"))?,
    )?;
    fs::copy(
        fixtures.join("relaxed-contracts.mjs"),
        input.join("contracts.mjs"),
    )?;
    let reference = Command::new("node")
        .arg(fixture_root().join("wasm-reference.mjs"))
        .arg(&input)
        .output()?;
    assert!(
        reference.status.success(),
        "{}",
        String::from_utf8_lossy(&reference.stderr)
    );
    let expected: serde_json::Value = serde_json::from_slice(&reference.stdout)?;
    assert_eq!(expected["relaxedSimd"], true, "workerd: {expected}");
    let worker = module_worker(&input, "contracts.mjs", &directory.path().join("app"))?;
    let actual: serde_json::Value = serde_json::from_slice(&request(&worker, "")?)?;
    assert_eq!(actual, expected);
    Ok(())
}

#[test]
fn wasm_feature_compatibility_audit() -> TestResult {
    let cases = [
        (
            "simd",
            "(module (func (result v128) v128.const i32x4 0 0 0 0))",
        ),
        ("tailCalls", "(module (func $f) (func return_call $f))"),
        (
            "extendedConstants",
            "(module (global i32 (i32.add (i32.const 1) (i32.const 2))))",
        ),
        ("sharedMemory", "(module (memory 1 2 shared))"),
        ("memory64", "(module (memory i64 1))"),
        ("table64", "(module (table i64 1 funcref))"),
        ("multipleMemories", "(module (memory 1) (memory 1))"),
        ("exceptions", "(module (tag $e) (func throw $e))"),
        ("gc", "(module (type (struct (field i32))))"),
        (
            "typedFunctions",
            "(module (type $f (func)) (func (param (ref $f)) local.get 0 call_ref $f))",
        ),
        (
            "relaxedSimd",
            "(module (func (param v128 v128) (result v128) local.get 0 local.get 1 i8x16.relaxed_swizzle))",
        ),
    ];
    let modules = cases
        .into_iter()
        .map(|(name, wat)| Ok((name, wat::parse_str(wat)?)))
        .collect::<Result<std::collections::BTreeMap<_, _>, wat::Error>>()?;
    let source = format!(
        r"
        const modules = {};
        export default {{ fetch() {{
            const binary = Object.fromEntries(Object.entries(modules).map(([name, bytes]) => [name, WebAssembly.validate(new Uint8Array(bytes))]));
            const api = Object.fromEntries(['Tag', 'Exception', 'Suspending', 'promising'].map(name => [name, typeof WebAssembly[name] === 'function']));
            api.resizableMemory = typeof WebAssembly.Memory.prototype.toResizableBuffer === 'function';
            return Response.json({{ binary, api }});
        }} }};
    ",
        serde_json::to_string(&modules)?
    );
    let directory = tempfile::tempdir()?;
    let input = directory.path().join("input");
    fs::create_dir(&input)?;
    fs::write(input.join("contracts.mjs"), source)?;
    let reference = Command::new("node")
        .arg(fixture_root().join("wasm-reference.mjs"))
        .arg(&input)
        .output()?;
    assert!(
        reference.status.success(),
        "{}",
        String::from_utf8_lossy(&reference.stderr)
    );
    let expected: serde_json::Value = serde_json::from_slice(&reference.stdout)?;
    let worker = module_worker(&input, "contracts.mjs", &directory.path().join("app"))?;
    let actual: serde_json::Value = serde_json::from_slice(&request(&worker, "")?)?;
    eprintln!("Wasm feature audit: workerd={expected}; tokamak={actual}");
    let supported = [
        "simd",
        "tailCalls",
        "extendedConstants",
        "sharedMemory",
        "relaxedSimd",
    ];
    for name in modules.keys() {
        assert_eq!(expected["binary"][*name], true, "workerd {name}");
        assert_eq!(
            actual["binary"][*name],
            supported.contains(name),
            "update the documented gap for {name}"
        );
    }
    Ok(())
}
