use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use tokio_util::sync::CancellationToken;

use super::{Registry, Service, Target};
use crate::quickjs::WorkerBundle;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn service(
    source: &str,
    name: &str,
    root: &std::path::Path,
) -> Result<Service, Box<dyn std::error::Error>> {
    let directory = root.join(name);
    std::fs::create_dir_all(&directory)?;
    let mut config = crate::tests::runtime_config();
    config.environment.insert("NAME".into(), name.into());
    Ok(Service {
        bundle: WorkerBundle::of_source(source, &directory)?,
        config,
        bindings: BTreeMap::new(),
    })
}

fn run(client: &str, server: &str) -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut main = service(client, "main", directory.path())?;
    let mut device = service(server, "device", directory.path())?;
    main.bindings
        .insert("DEVICE".into(), Target::new("device", "default"));
    main.bindings
        .insert("NAMED".into(), Target::new("device", "Named"));
    device
        .bindings
        .insert("MAIN".into(), Target::new("main", "default"));
    let registry = Arc::new(Registry::new(BTreeMap::from([
        ("main".into(), main),
        ("device".into(), device),
    ]))?);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let scope = super::Scope::new(&CancellationToken::new());
        let call = registry.invoke(
            Target::new("main", "default"),
            "run".into(),
            super::Message::empty_arguments(),
            scope.lifetime.clone(),
        );
        let result = tokio::time::timeout(Duration::from_secs(5), call).await??;
        assert!(!result.failed, "{}", result.value.data);
        Ok::<_, Box<dyn std::error::Error>>(())
    })
}

#[test]
fn service_methods_preserve_framework_reserved_names() -> TestResult {
    run(
        r"
import { WorkerEntrypoint } from 'cloudflare:workers';
import assert from 'node:assert/strict';
export default class extends WorkerEntrypoint {
  async run() {
    for (const name of [
      'dup', 'alarm', 'webSocketMessage', 'webSocketClose', 'webSocketError',
      'queue', 'scheduled', 'email', 'tail', 'trace', 'tailStream', 'test', '__tokamakRpc',
    ]) {
      await assert.rejects(this.env.DEVICE[name](), { name: 'TypeError' });
    }
    assert.equal(await this.env.DEVICE.ping(), 'pong');
  }
}
",
        r"
import { WorkerEntrypoint } from 'cloudflare:workers';
export default class extends WorkerEntrypoint {
  dup() { return 'reserved'; }
  alarm() { return 'reserved'; }
  webSocketMessage() { return 'reserved'; }
  webSocketClose() { return 'reserved'; }
  webSocketError() { return 'reserved'; }
  queue() { return 'reserved'; }
  scheduled() { return 'reserved'; }
  email() { return 'reserved'; }
  tail() { return 'reserved'; }
  trace() { return 'reserved'; }
  tailStream() { return 'reserved'; }
  test() { return 'reserved'; }
  __tokamakRpc() { return 'reserved'; }
  ping() { return 'pong'; }
}
",
    )
}

#[test]
fn service_calls_use_the_receivers_bindings_and_isolated_state() -> TestResult {
    run(
        r"
import { WorkerEntrypoint } from 'cloudflare:workers';
import assert from 'node:assert/strict';
import { writeFileSync, existsSync } from 'node:fs';
export default class extends WorkerEntrypoint {
  async run() {
    writeFileSync('/tmp/owner', 'main');
    assert.deepEqual(await this.env.DEVICE.describe(), { name: 'device', calls: 1, file: false, fromMain: 'main' });
    assert.deepEqual(await this.env.DEVICE.describe(), { name: 'device', calls: 1, file: false, fromMain: 'main' });
    assert.equal(await this.env.NAMED.describe(), 'named device');
    assert.equal(existsSync('/tmp/owner'), true);
  }
  name() { return this.env.NAME; }
}
",
        r"
import { WorkerEntrypoint } from 'cloudflare:workers';
import { existsSync } from 'node:fs';
let calls = 0;
export default class extends WorkerEntrypoint {
  async describe() { return { name: this.env.NAME, calls: ++calls, file: existsSync('/tmp/owner'), fromMain: await this.env.MAIN.name() }; }
}
export class Named extends WorkerEntrypoint {
  describe() { return 'named ' + this.env.NAME; }
}
",
    )
}

#[test]
fn service_values_preserve_graphs_bytes_and_errors_between_runtimes() -> TestResult {
    run(
        r"
import { WorkerEntrypoint } from 'cloudflare:workers';
import assert from 'node:assert/strict';
export default class extends WorkerEntrypoint {
  async run() {
    const buffer = new ArrayBuffer(8);
    const value = { bytes: new Uint8Array(buffer), view: new DataView(buffer), date: new Date(1000), numbers: [undefined, NaN, Infinity, -0, 2n ** 90n], map: new Map() };
    value.self = value;
    value.map.set(value, new Set([value]));
    const result = await this.env.DEVICE.echo(value);
    assert.equal(result.self, result);
    assert.equal(result.bytes.buffer, result.view.buffer);
    assert.equal(result.bytes[0], 42);
    assert.equal(value.bytes[0], 0);
    assert.equal(result.map.get(result).has(result), true);
    assert.deepEqual(result.numbers, value.numbers);
    assert.equal(result.date.getTime(), 1000);
    const error = await this.env.DEVICE.fail().then(() => assert.fail('Expected rejection'), error => error);
    assert.equal(error.name, 'DeviceError');
    assert.equal(error.message, 'offline');
    assert.equal(error.code, 'OFFLINE');
    assert.equal(error.cause.name, 'TypeError');
    assert.equal(error.cause.message, 'bad input');
    await assert.rejects(this.env.DEVICE.echo({ callback() {} }), { name: 'DataCloneError' });
    await assert.rejects(this.env.DEVICE.invalidResult(), { name: 'DataCloneError' });
  }
}
",
        r"
import { WorkerEntrypoint } from 'cloudflare:workers';
export default class extends WorkerEntrypoint {
  echo(value) { value.bytes[0] = 42; return value; }
  fail() { throw Object.assign(new Error('offline', { cause: new TypeError('bad input') }), { name: 'DeviceError', code: 'OFFLINE' }); }
  invalidResult() { return () => {}; }
}
",
    )
}

#[test]
fn service_fetch_preserves_http_metadata_and_streams_request_and_response_bodies() -> TestResult {
    run(
        r"
import { WorkerEntrypoint } from 'cloudflare:workers';
import assert from 'node:assert/strict';
export default class extends WorkerEntrypoint {
  async run() {
    const input = new ReadableStream({ start(c) { c.enqueue(new TextEncoder().encode('streamed upload')); c.close(); } });
    const response = await this.env.DEVICE.fetch('https://device.example/upload?q=1', {
      method: 'POST', headers: { 'x-caller': 'main' }, body: input,
    });
    assert.equal(response.status, 201);
    assert.equal(response.statusText, 'Created locally');
    assert.deepEqual(response.headers.getSetCookie(), ['one=1', 'two=2']);
    assert.equal(response.headers.get('x-receiver'), 'device');
    assert.equal(await response.text(), 'POST /upload?q=1 main streamed upload');
    const named = await this.env.NAMED.fetch('https://device.example/');
    assert.equal(await named.text(), 'named device');
    await assert.rejects(this.env.DEVICE.fetch('https://device.example/fail'), { name: 'TypeError', message: 'fetch failed' });
    await assert.rejects(this.env.DEVICE.fetch('https://device.example/upgrade'), { name: 'TypeError' });
    const active = new AbortController();
    const pending = await this.env.DEVICE.fetch('https://device.example/pending', { signal: active.signal });
    const reading = pending.body.getReader().read();
    active.abort();
    await assert.rejects(reading, { name: 'AbortError' });
    const controller = new AbortController();
    controller.abort(new Error('cancelled by caller'));
    await assert.rejects(this.env.DEVICE.fetch('https://device.example/', { signal: controller.signal }), { message: 'cancelled by caller' });
  }
}
",
        r"
import { WorkerEntrypoint } from 'cloudflare:workers';
export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    if (url.pathname === '/fail') throw new TypeError('fetch failed');
    if (url.pathname === '/upgrade') return new Response(null, { status: 101, webSocket: new WebSocketPair()[0] });
    if (url.pathname === '/pending') return new Response(new ReadableStream({ pull() { return new Promise(() => {}); } }));
    const body = `${request.method} ${url.pathname}${url.search} ${request.headers.get('x-caller')} ${await request.text()}`;
    return new Response(body, { status: 201, statusText: 'Created locally', headers: [['set-cookie', 'one=1'], ['set-cookie', 'two=2'], ['x-receiver', env.NAME]] });
  },
};
export class Named extends WorkerEntrypoint {
  fetch() { return new Response('named ' + this.env.NAME); }
}
",
    )
}

#[test]
fn byte_streams_transfer_in_both_directions_with_cancellation_and_errors() -> TestResult {
    run(
        r"
import { WorkerEntrypoint } from 'cloudflare:workers';
import assert from 'node:assert/strict';
export default class extends WorkerEntrypoint {
  async run() {
    let cancelled;
    const input = new ReadableStream({
      pull(c) { c.enqueue(new Uint8Array([1, 2, 3])); },
      cancel(reason) { cancelled = reason; },
    }, { highWaterMark: 0 });
    assert.deepEqual(await this.env.DEVICE.consume({ input }), [1, 2, 3]);
    assert.equal(input.locked, true);
    assert.equal(cancelled.message, 'enough');
    const { output } = await this.env.DEVICE.download();
    const reader = output.getReader();
    let length = 0;
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      assert.ok(value.length <= 65536);
      length += value.length;
      assert.ok(value.every(byte => byte === 7));
    }
    assert.equal(length, 70000);
    const failed = (await this.env.DEVICE.fail()).getReader();
    await assert.rejects(failed.read(), { name: 'TypeError', message: 'source failed' });
    const pending = (await this.env.DEVICE.pending()).getReader();
    const read = pending.read();
    await pending.cancel('stop');
    assert.equal((await read).done, true);
    const invalid = (await this.env.DEVICE.invalid()).getReader();
    await assert.rejects(invalid.read(), { name: 'DataCloneError' });
  }
}
",
        r"
import { WorkerEntrypoint } from 'cloudflare:workers';
export default class extends WorkerEntrypoint {
  async consume({ input }) {
    const reader = input.getReader();
    const { value } = await reader.read();
    await reader.cancel(new Error('enough'));
    return [...value];
  }
  download() {
    return { output: new ReadableStream({ start(c) { c.enqueue(new Uint8Array(70000).fill(7)); c.close(); } }) };
  }
  fail() { return new ReadableStream({ pull(c) { c.error(new TypeError('source failed')); } }); }
  pending() { return new ReadableStream({ pull() { return new Promise(() => {}); } }); }
  invalid() { return new ReadableStream({ start(c) { c.enqueue('not bytes'); }, cancel() { return new Promise(() => {}); } }); }
}
",
    )
}

#[test]
fn service_methods_expose_only_public_prototype_operations() -> TestResult {
    run(
        r"
import { WorkerEntrypoint } from 'cloudflare:workers';
import assert from 'node:assert/strict';
export default class extends WorkerEntrypoint {
  async run() {
    for (const method of ['event', 'constructor', 'connect', 'toString', 'secret', 'getter', 'own']) {
      await assert.rejects(this.env.DEVICE[method](), { name: 'TypeError' });
    }
    assert.equal(await this.env.DEVICE.public(), 42);
  }
}
",
        r"
import { WorkerEntrypoint } from 'cloudflare:workers';
export default class extends WorkerEntrypoint {
  own = () => 5;
  event() { return 1; }
  #secret() { return 42; }
  get getter() { return () => 3; }
  public() { return this.#secret(); }
}
",
    )
}

struct Gate(flume::Sender<(u64, String)>);

impl crate::PluginHandler for Gate {
    fn call(&self, id: u64, _: &str, method: &str, _: &str) {
        let _ = self.0.send((id, method.to_owned()));
    }
    fn subscribe(&self, _: u64, _: &str, _: &str, _: &str) {}
    fn cancel(&self, _: u64) {}
}

#[test]
fn wait_until_continues_after_the_caller_receives_its_result() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut main = service(
        r"
import { WorkerEntrypoint } from 'cloudflare:workers';
export default class extends WorkerEntrypoint {
  async run() { return await this.env.DEVICE.start(); }
}
",
        "main",
        directory.path(),
    )?;
    let mut device = service(
        r"
import { WorkerEntrypoint } from 'cloudflare:workers';
export default class extends WorkerEntrypoint {
  start() {
    this.ctx.waitUntil((async () => {
      await globalThis.__tokamakNativeCall('gate', 'wait');
      await globalThis.__tokamakNativeCall('gate', 'complete');
    })());
    return 'accepted';
  }
}
",
        "device",
        directory.path(),
    )?;
    main.bindings
        .insert("DEVICE".into(), Target::new("device", "default"));
    let (sent, received) = flume::unbounded();
    let plugins = crate::plugin_calls::PluginCalls::new(Some(Arc::new(Gate(sent))), true);
    device.config.plugins = Some(Arc::clone(&plugins));
    let registry = Arc::new(Registry::new(BTreeMap::from([
        ("main".into(), main),
        ("device".into(), device),
    ]))?);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let stopped = CancellationToken::new();
        let scope = super::Scope::new(&stopped);
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            registry.invoke(
                Target::new("main", "default"),
                "run".into(),
                super::Message::empty_arguments(),
                scope.lifetime.clone(),
            ),
        )
        .await??;
        assert!(!result.failed, "{}", result.value.data);
        assert!(result.value.data.contains("accepted"));
        drop(scope);
        let (waiting, method) =
            tokio::time::timeout(Duration::from_secs(5), received.recv_async()).await??;
        assert_eq!(method, "wait");
        plugins.reply(waiting, r#"{"value":null,"done":true}"#);
        let (complete, method) =
            tokio::time::timeout(Duration::from_secs(5), received.recv_async()).await??;
        assert_eq!(method, "complete");
        plugins.reply(complete, r#"{"value":null,"done":true}"#);
        stopped.cancel();
        Ok::<_, Box<dyn std::error::Error>>(())
    })
}

#[test]
fn an_inherited_deadline_interrupts_cpu_bound_service_code() -> TestResult {
    let directory = tempfile::tempdir()?;
    let worker = service(
        r"
import { WorkerEntrypoint } from 'cloudflare:workers';
export default class extends WorkerEntrypoint { run() { while (true) {} } }
",
        "device",
        directory.path(),
    )?;
    let registry = Arc::new(Registry::new(BTreeMap::from([("device".into(), worker)]))?);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let scope = super::Scope::until(
            &CancellationToken::new(),
            std::time::Instant::now() + Duration::from_millis(100),
        );
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            registry.invoke(
                Target::new("device", "default"),
                "run".into(),
                super::Message::empty_arguments(),
                scope.lifetime.clone(),
            ),
        )
        .await?;
        assert!(result.is_err());
        assert!(
            scope.lifetime.expired(),
            "execution ended before its deadline"
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })
}

#[test]
fn caller_cancellation_preserves_registered_wait_until_work() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut worker = service(
        r"
import { WorkerEntrypoint } from 'cloudflare:workers';
export default class extends WorkerEntrypoint {
  async run() {
    this.ctx.waitUntil((async () => {
      await globalThis.__tokamakNativeCall('gate', 'retained');
      await globalThis.__tokamakNativeCall('gate', 'complete');
    })());
    await globalThis.__tokamakNativeCall('gate', 'handler');
  }
}
",
        "device",
        directory.path(),
    )?;
    let (sent, received) = flume::unbounded();
    let plugins = crate::plugin_calls::PluginCalls::new(Some(Arc::new(Gate(sent))), true);
    worker.config.plugins = Some(Arc::clone(&plugins));
    let registry = Arc::new(Registry::new(BTreeMap::from([("device".into(), worker)]))?);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let stopped = CancellationToken::new();
        let _shutdown = stopped.clone().drop_guard();
        let scope = super::Scope::new(&stopped);
        let lifetime = scope.lifetime.clone();
        let call = tokio::spawn(async move {
            registry
                .invoke(
                    Target::new("device", "default"),
                    "run".into(),
                    super::Message::empty_arguments(),
                    lifetime,
                )
                .await
        });
        let mut pending = BTreeMap::new();
        for _ in 0..2 {
            let (id, method) =
                tokio::time::timeout(Duration::from_secs(5), received.recv_async()).await??;
            pending.insert(method, id);
        }
        drop(scope);
        assert!(
            tokio::time::timeout(Duration::from_secs(5), call)
                .await??
                .is_err()
        );
        plugins.reply(pending["retained"], r#"{"value":null,"done":true}"#);
        let (complete, method) =
            tokio::time::timeout(Duration::from_secs(5), received.recv_async()).await??;
        assert_eq!(method, "complete");
        plugins.reply(complete, r#"{"value":null,"done":true}"#);
        Ok::<_, Box<dyn std::error::Error>>(())
    })
}

#[test]
fn caller_cancellation_completes_rpc_stream_cleanup() -> TestResult {
    check_rpc_stream_cleanup(false, false)
}

#[test]
fn dropping_the_returned_stream_completes_its_cancellation_cleanup() -> TestResult {
    check_rpc_stream_cleanup(false, true)
}

#[test]
fn host_deadline_ends_blocked_rpc_stream_cleanup() -> TestResult {
    check_rpc_stream_cleanup(true, false)
}

fn check_rpc_stream_cleanup(expire: bool, drop_stream: bool) -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut worker = service(
        r"
import { WorkerEntrypoint } from 'cloudflare:workers';
export default class extends WorkerEntrypoint {
  run() {
    return new ReadableStream({
      type: 'bytes',
      async cancel() {
        let sum = 0;
        for (let i = 0; i < 50000; i++) sum += i;
        if (sum !== 1249975000) throw new Error('cleanup calculation failed');
        await globalThis.__tokamakNativeCall('gate', 'cleanup');
        await globalThis.__tokamakNativeCall('gate', 'complete');
      },
    });
  }
}
",
        "device",
        directory.path(),
    )?;
    let (sent, received) = flume::unbounded();
    let plugins = crate::plugin_calls::PluginCalls::new(Some(Arc::new(Gate(sent))), true);
    worker.config.plugins = Some(Arc::clone(&plugins));
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let stopped = CancellationToken::new();
        let _shutdown = stopped.clone().drop_guard();
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        let scope = super::Scope::until(&stopped, deadline);
        let lifetime = scope.lifetime.clone();
        let (sender, responses) = flume::bounded(1);
        let execution = tokio::task::spawn_blocking(move || {
            tokio::runtime::Handle::current().block_on(crate::dispatcher::execute_service(
                &worker.bundle,
                &worker.config,
                super::Call {
                    entrypoint: "default".into(),
                    method: "run".into(),
                    arguments: super::Message::empty_arguments(),
                },
                &lifetime,
                &sender,
            ))
        });
        let response =
            tokio::time::timeout(Duration::from_secs(2), responses.recv_async()).await???;
        assert!(!response.failed);
        let mut response = Some(response);
        drop(scope);
        if drop_stream {
            drop(response.take());
        }
        let (cleanup, method) =
            tokio::time::timeout(Duration::from_secs(2), received.recv_async()).await??;
        assert_eq!(method, "cleanup");
        assert!(
            !execution.is_finished(),
            "stream cleanup must keep its invocation alive"
        );
        if !expire {
            plugins.reply(cleanup, r#"{"value":null,"done":true}"#);
            let (complete, method) =
                tokio::time::timeout(Duration::from_secs(2), received.recv_async()).await??;
            assert_eq!(method, "complete");
            plugins.reply(complete, r#"{"value":null,"done":true}"#);
        }
        tokio::time::timeout(Duration::from_secs(2), execution).await???;
        if expire {
            assert!(
                std::time::Instant::now() >= deadline,
                "cleanup ended before its deadline"
            );
        }
        drop(response);
        Ok::<_, Box<dyn std::error::Error>>(())
    })
}

#[test]
fn an_inherited_deadline_finishes_wait_until_blocked_on_native_io() -> TestResult {
    let directory = tempfile::tempdir()?;
    let mut worker = service(
        r"
import { WorkerEntrypoint } from 'cloudflare:workers';
export default class extends WorkerEntrypoint {
  run() {
    this.ctx.waitUntil(globalThis.__tokamakNativeCall('gate', 'retained'));
    return new ReadableStream({ pull() { return new Promise(() => {}); } });
  }
}
",
        "device",
        directory.path(),
    )?;
    let (sent, received) = flume::unbounded();
    let plugins = crate::plugin_calls::PluginCalls::new(Some(Arc::new(Gate(sent))), true);
    worker.config.plugins = Some(plugins);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let stopped = CancellationToken::new();
        let _shutdown = stopped.clone().drop_guard();
        let scope = super::Scope::until(
            &stopped,
            std::time::Instant::now() + Duration::from_millis(500),
        );
        let lifetime = scope.lifetime.clone();
        let (sender, responses) = flume::bounded(1);
        let execution = tokio::task::spawn_blocking(move || {
            tokio::runtime::Handle::current().block_on(crate::dispatcher::execute_service(
                &worker.bundle,
                &worker.config,
                super::Call {
                    entrypoint: "default".into(),
                    method: "run".into(),
                    arguments: super::Message::empty_arguments(),
                },
                &lifetime,
                &sender,
            ))
        });
        let response =
            tokio::time::timeout(Duration::from_secs(2), responses.recv_async()).await???;
        assert!(!response.failed);
        let (_, method) =
            tokio::time::timeout(Duration::from_secs(2), received.recv_async()).await??;
        assert_eq!(method, "retained");
        drop(scope);
        tokio::time::timeout(Duration::from_secs(2), execution).await???;
        drop(response);
        Ok::<_, Box<dyn std::error::Error>>(())
    })
}
