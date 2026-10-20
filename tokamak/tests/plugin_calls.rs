//! The Worker's calls on the shell's plugins and on the runtime itself.
#![cfg(feature = "native")]

use std::collections::BTreeMap;
use std::fs;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use serde_json::{Value, json};
use tokamak::{
    Config, ModuleType, PackageLayout, PluginHandler, PluginReply, Runtime, WorkerEnvironment,
    WorkerManifest, write_worker, write_worker_environment,
};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Runs the method of `TokamakEvents` each event names.
const WORKER: &[u8] = br#"const call = async (ctx, plugin, method, value = null) => JSON.parse(new TextDecoder().decode(await ctx[Symbol.for("tokamak.native")].call(plugin, method, new TextEncoder().encode(JSON.stringify(value)))));
const nativeListen = (ctx, plugin, method, value, next, error) => ctx[Symbol.for("tokamak.native")].listen(plugin, method, new TextEncoder().encode(JSON.stringify(value)), { next: payload => next(JSON.parse(new TextDecoder().decode(payload))), error });

import { WorkerEntrypoint } from "cloudflare:workers";

const failure = (error) => ({ name: error.name, message: error.message, domException: error instanceof DOMException });
const sleep = (milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds));

export class TokamakEvents extends WorkerEntrypoint {
  async dispatch(name, payload) {
    const event = JSON.parse(new TextDecoder().decode(payload));
    const reply = name === "start" || name === "suspend" ? undefined : await this[name](event);
    return {
      reply: reply === undefined ? new Uint8Array() : new TextEncoder().encode(JSON.stringify(reply)),
      listened: ["start", "resume", "suspend", "call", "stage", "watch", "forget", "raw", "upload", "abort"],
    };
  }

  async resume(event) {
    void call(this.ctx, "test", "resuming");
    await sleep(event.wait);
  }

  call(event) {
    return call(this.ctx, event.plugin, event.method, event.arguments).catch(failure);
  }

  stage() {
    return call(this.ctx, "tokamak", "lifecycleStage");
  }

  async raw(event) {
    const input = Uint8Array.from(event.bytes);
    const output = await this.ctx[Symbol.for("tokamak.native")].call("bytes", "echo", input.subarray(1, 5));
    return Array.from(output);
  }

  async upload(event) {
    const body = new ReadableStream({
      start(controller) {
        for (const chunk of event.chunks) controller.enqueue(new Uint8Array(chunk));
        controller.close();
      },
    });
    const reply = await this.ctx[Symbol.for("tokamak.native")].call("files", "write", new TextEncoder().encode("{}"), { body });
    return new TextDecoder().decode(reply);
  }

  async abort() {
    const controller = new AbortController();
    const pending = this.ctx[Symbol.for("tokamak.native")].call("test", "hang", new Uint8Array(), { signal: controller.signal });
    setTimeout(() => controller.abort(new DOMException("stopped", "AbortError")));
    return pending.then(() => "resolved", error => error.message);
  }

  // Reports each position with `test.received`, discarding the function that
  // removes the listener.
  forget() {
    nativeListen(this.ctx, "location", "watchPosition", null, (position) => {
      void call(this.ctx, "test", "received", position);
    }, () => undefined);
    this.ctx.waitUntil(call(this.ctx, "test", "retained"));
  }

  watch() {
    this.ctx.waitUntil(new Promise(resolve => {
      const stop = nativeListen(this.ctx, "location", "watchPosition", null, (position) => {
        if (position !== "stop") return void call(this.ctx, "test", "received", position);
        stop();
        this.ctx.waitUntil(call(this.ctx, "test", "stopped"));
        resolve();
        setTimeout(() => void call(this.ctx, "test", "alive"), 500);
      }, (error) => {
        void call(this.ctx, "test", "failed", failure(error));
        if (error.name === "NotSupportedError") resolve();
      });
    }));
  }

}

export default {
  fetch(request, env, ctx) {
    nativeListen(ctx, "location", "watchPosition", null, () => {}, () => {});
    ctx.waitUntil(call(ctx, "test", "retained"));
    return new Response(null, { status: 204 });
  }
};
"#;

const TIMEOUT: Duration = Duration::from_secs(5);

/// A request the runtime made of the shell's plugins.
#[derive(Debug)]
struct Request {
    id: u64,
    /// The operation, plugin, method and arguments, or `cancel`.
    text: String,
    payload: Vec<u8>,
}

/// Shell plugins whose requests the test answers.
struct Shell(Sender<Request>);

impl Shell {
    fn record(&self, id: u64, text: String, payload: &[u8]) {
        let _ = self.0.send(Request {
            id,
            text,
            payload: payload.to_vec(),
        });
    }
}

impl PluginHandler for Shell {
    fn call(&self, id: u64, plugin: &str, method: &str, arguments: &[u8]) {
        let text = String::from_utf8_lossy(arguments);
        self.record(id, format!("call {plugin}.{method} {text}"), arguments);
    }

    fn call_with_body(&self, id: u64, plugin: &str, method: &str, arguments: &[u8]) {
        let text = String::from_utf8_lossy(arguments);
        self.record(id, format!("body {plugin}.{method} {text}"), arguments);
    }

    fn chunk(&self, id: u64, chunk: &PluginReply) {
        let state = if chunk.failed {
            "failed"
        } else if chunk.done {
            "done"
        } else {
            "more"
        };
        self.record(id, format!("chunk {state}"), &chunk.payload);
    }

    fn subscribe(&self, id: u64, plugin: &str, method: &str, arguments: &[u8]) {
        let text = String::from_utf8_lossy(arguments);
        self.record(id, format!("subscribe {plugin}.{method} {text}"), arguments);
    }

    fn cancel(&self, id: u64) {
        self.record(id, "cancel".to_owned(), &[]);
    }
}

/// A started runtime, and the requests its Worker makes of the shell's
/// plugins.
struct Started {
    runtime: Arc<Runtime>,
    requests: Receiver<Request>,
    _directory: tempfile::TempDir,
}

impl Started {
    /// The runtime of `WORKER`, with the shell's plugins unless `plugins` is
    /// false.
    fn new(plugins: bool) -> TestResult<Self> {
        let directory = tempfile::tempdir()?;
        let app = PackageLayout::new(directory.path().join("app"));
        let native = app.tokamak_worker();
        fs::create_dir_all(native.root())?;
        write_worker_environment(&native, &WorkerEnvironment::default())?;
        fs::write(directory.path().join("worker.mjs"), WORKER)?;
        write_worker(
            &native,
            directory.path(),
            &WorkerManifest {
                entry: "worker.mjs".to_owned(),
                modules: BTreeMap::from([("worker.mjs".to_owned(), ModuleType::EsModule)]),
            },
        )?;
        write_worker_environment(
            &app,
            &WorkerEnvironment {
                services: BTreeMap::from([(
                    "TOKAMAK".into(),
                    tokamak::ServiceBinding {
                        service: "tokamak".into(),
                        entrypoint: "default".into(),
                    },
                )]),
                ..WorkerEnvironment::default()
            },
        )?;
        fs::write(
            directory.path().join("worker.mjs"),
            "export default { fetch: (request, env) => env.TOKAMAK.fetch(request) };",
        )?;
        write_worker(
            &app,
            directory.path(),
            &WorkerManifest {
                entry: "worker.mjs".into(),
                modules: BTreeMap::from([("worker.mjs".into(), ModuleType::EsModule)]),
            },
        )?;
        let (sender, requests) = mpsc::channel();
        let runtime = Runtime::start(
            Config {
                app,
                state_dir: directory.path().join("state"),
                storage_dir: directory.path().join("storage"),
                host: "app.tokamak.local".to_owned(),
                foreground: true,
                plugins: plugins.then(|| Arc::new(Shell(sender)) as Arc<dyn PluginHandler>),
            },
            |_| {},
        )?;
        Ok(Self {
            runtime: Arc::new(runtime),
            requests,
            _directory: directory,
        })
    }

    /// Delivers the event `name` on another thread, returning its JSON reply,
    /// or null without one, once it completes within `timeout`.
    fn emit(&self, name: &str, event: &Value, timeout: Duration) -> JoinHandle<TestResult<Value>> {
        let runtime = Arc::clone(&self.runtime);
        let (name, event) = (name.to_owned(), event.to_string());
        thread::spawn(move || {
            let reply = runtime.emit(&name, event.as_bytes(), timeout)?;
            if reply.is_empty() {
                return Ok(Value::Null);
            }
            Ok(serde_json::from_slice(&reply)?)
        })
    }

    /// The reply to the event `name`.
    fn reply_to(&self, name: &str, event: &Value) -> TestResult<Value> {
        join(self.emit(name, event, TIMEOUT))
    }

    fn next_request(&self) -> TestResult<Request> {
        Ok(self.requests.recv_timeout(TIMEOUT)?)
    }

    fn acknowledge(&self, request: &Request) {
        self.runtime.reply(request.id, value(b"null", true));
    }
}

fn join<T>(thread: JoinHandle<TestResult<T>>) -> TestResult<T> {
    thread.join().map_err(|_| "the thread panicked")?
}

fn call_event(plugin: &str, method: &str, arguments: &Value) -> Value {
    json!({ "plugin": plugin, "method": method, "arguments": arguments })
}

#[test]
fn resolves_a_plugin_call_with_the_shell_reply() -> TestResult {
    let started = Started::new(true)?;

    let reply = started.emit(
        "call",
        &call_event("location", "getCurrentPosition", &json!({ "timeout": 1 })),
        TIMEOUT,
    );
    let request = started.next_request()?;
    started
        .runtime
        .reply(request.id, value(br#"{"latitude":51.5}"#, true));

    assert_eq!(
        request.text,
        r#"call location.getCurrentPosition {"timeout":1}"#
    );
    assert_eq!(join(reply)?, json!({ "latitude": 51.5 }));
    Ok(())
}

#[test]
fn rejects_a_plugin_call_with_the_exception_the_shell_names() -> TestResult {
    let started = Started::new(true)?;

    let reply = started.emit(
        "call",
        &call_event("notifications", "requestPermission", &Value::Null),
        TIMEOUT,
    );
    let request = started.next_request()?;
    started.runtime.reply(
        request.id,
        tokamak::PluginReply {
            payload: br#"{"name":"NeedsUIError","message":"The app is in the background"}"#
                .to_vec(),
            failed: true,
            done: true,
        },
    );

    assert_eq!(
        join(reply)?,
        json!({ "name": "NeedsUIError", "message": "The app is in the background", "domException": true })
    );
    Ok(())
}

#[test]
fn answers_tokamak_calls_without_the_shell_plugins() -> TestResult {
    let started = Started::new(false)?;

    assert_eq!(started.reply_to("stage", &json!({}))?, "foreground");
    assert!(started.runtime.is_foreground());
    assert!(started.runtime.set_foreground(false));
    assert!(!started.runtime.set_foreground(false));
    assert!(!started.runtime.is_foreground());
    assert_eq!(started.reply_to("stage", &json!({}))?, "background");
    let failure = started.reply_to(
        "call",
        &call_event("location", "getCurrentPosition", &Value::Null),
    )?;
    assert_eq!(failure["name"], "NotSupportedError");
    assert_eq!(failure["domException"], true);
    Ok(())
}

#[test]
fn records_the_stage_while_a_lifecycle_event_runs() -> TestResult {
    let started = Started::new(true)?;
    let resume = started.emit("resume", &json!({ "wait": 1000 }), TIMEOUT);
    assert_eq!(started.next_request()?.text, "call test.resuming null");

    started.runtime.set_foreground(false);

    assert_eq!(started.reply_to("stage", &json!({}))?, "background");
    assert!(!resume.is_finished());
    join(resume)?;
    Ok(())
}

#[test]
fn routes_core_device_operations_to_the_shell() -> TestResult {
    let started = Started::new(true)?;
    let event = started.emit(
        "call",
        &json!({
            "plugin": "tokamak", "method": "webview.getCurrentPage", "arguments": null,
        }),
        TIMEOUT,
    );
    let request = started.next_request()?;
    assert_eq!(request.text, "call tokamak.webview.getCurrentPage null");
    started.runtime.reply(
        request.id,
        value(br#""https://app.tokamak.local/recipes""#, true),
    );
    assert_eq!(join(event)?, json!("https://app.tokamak.local/recipes"));
    Ok(())
}

#[test]
fn delivers_listener_values_and_errors_during_registered_work() -> TestResult {
    let started = Started::new(true)?;
    let event = started.emit("watch", &json!({}), TIMEOUT);
    let subscription = started.next_request()?;
    assert_eq!(subscription.text, "subscribe location.watchPosition null");

    let deliver = |result: tokamak::PluginReply| started.runtime.reply(subscription.id, result);
    deliver(value(b"1", false));
    let received = started.next_request()?;
    assert_eq!(received.text, "call test.received 1");
    started.acknowledge(&received);
    deliver(tokamak::PluginReply {
        payload: br#"{"name":"NotReadableError","message":"No fix"}"#.to_vec(),
        failed: true,
        done: false,
    });
    let failed = started.next_request()?;
    assert_eq!(
        failed.text,
        r#"call test.failed {"name":"NotReadableError","message":"No fix","domException":true}"#
    );
    started.acknowledge(&failed);
    deliver(value(br#""stop""#, false));

    let mut requests = [started.next_request()?, started.next_request()?];
    requests.sort_by_key(|request| request.text.clone());
    let [stopped, cancelled] = requests;
    assert_eq!(stopped.text, "call test.stopped null");
    assert_eq!(
        (cancelled.id, cancelled.text.as_str()),
        (subscription.id, "cancel")
    );
    started.runtime.reply(stopped.id, value(b"null", true));
    assert_eq!(join(event)?, Value::Null);
    let alive = started.requests.recv_timeout(Duration::from_secs(1));
    assert!(alive.is_err(), "the context still ran: {alive:?}");
    Ok(())
}

#[test]
fn keeps_a_listener_whose_remove_function_is_discarded() -> TestResult {
    let started = Started::new(true)?;
    let event = started.emit("forget", &json!({}), TIMEOUT);
    let subscription = started.next_request()?;
    let retained = started.next_request()?;
    assert_eq!(retained.text, "call test.retained null");

    started.runtime.reply(subscription.id, value(b"1", false));

    let received = started.next_request()?;
    assert_eq!(received.text, "call test.received 1");
    started.acknowledge(&received);
    started.runtime.reply(retained.id, value(b"null", true));
    assert_eq!(join(event)?, Value::Null);
    let removed = started.next_request()?;
    assert_eq!(
        (removed.id, removed.text.as_str()),
        (subscription.id, "cancel")
    );
    Ok(())
}

#[test]
fn ends_a_listener_whose_result_is_done() -> TestResult {
    let started = Started::new(true)?;
    let event = started.emit("forget", &json!({}), TIMEOUT);
    let subscription = started.next_request()?;
    let retained = started.next_request()?;
    assert_eq!(retained.text, "call test.retained null");

    started.runtime.reply(subscription.id, value(b"1", true));
    let received = started.next_request()?;
    assert_eq!(received.text, "call test.received 1");
    started.acknowledge(&received);
    started.runtime.reply(subscription.id, value(b"2", false));

    let after = started.requests.recv_timeout(Duration::from_secs(1));
    assert!(after.is_err(), "the listener still ran: {after:?}");
    assert!(
        !event.is_finished(),
        "independent work must still be running"
    );
    started.runtime.reply(retained.id, value(b"null", true));
    assert_eq!(join(event)?, Value::Null);
    Ok(())
}

#[test]
fn cancels_calls_and_listeners_when_the_runtime_stops() -> TestResult {
    let started = Started::new(true)?;
    started.runtime.fetch("/", TIMEOUT)?;
    let subscription = started.next_request()?;
    let call = started.next_request()?;
    assert_eq!(call.text, "call test.retained null");

    let Started {
        runtime, requests, ..
    } = started;
    drop(Arc::into_inner(runtime).ok_or("the runtime is still shared")?);

    let mut cancelled = [
        requests.recv_timeout(TIMEOUT)?,
        requests.recv_timeout(TIMEOUT)?,
    ];
    cancelled.sort_by_key(|request| request.id);
    assert_eq!(
        cancelled.map(|request| (request.id, request.text)),
        [
            (subscription.id, "cancel".to_owned()),
            (call.id, "cancel".to_owned())
        ]
    );
    Ok(())
}

#[test]
fn event_deadlines_cancel_pending_native_calls() -> TestResult {
    let started = Started::new(true)?;
    let event = started.emit(
        "call",
        &call_event("location", "getCurrentPosition", &Value::Null),
        Duration::from_millis(300),
    );
    let call = started.next_request()?;
    assert_eq!(call.text, "call location.getCurrentPosition null");
    assert!(join(event).is_err());
    let cancelled = started.next_request()?;
    assert_eq!((cancelled.id, cancelled.text.as_str()), (call.id, "cancel"));
    started.runtime.reply(call.id, value(br#""late""#, true));
    assert_eq!(started.reply_to("stage", &json!({}))?, "foreground");
    Ok(())
}

#[test]
fn event_deadlines_release_listeners_and_later_events_get_fresh_invocations() -> TestResult {
    let started = Started::new(true)?;
    let mut previous = None;
    for _ in 0..2 {
        let event = started.emit("watch", &json!({}), Duration::from_millis(300));
        let subscription = started.next_request()?;
        assert_eq!(subscription.text, "subscribe location.watchPosition null");
        assert_ne!(Some(subscription.id), previous);
        assert_eq!(join(event)?, Value::Null);
        let removed = started.next_request()?;
        assert_eq!(
            (removed.id, removed.text.as_str()),
            (subscription.id, "cancel")
        );
        previous = Some(subscription.id);
    }
    Ok(())
}

#[test]
fn carries_binary_subarrays_between_quickjs_and_native_endpoints() -> TestResult {
    let started = Started::new(true)?;
    let event = started.emit(
        "raw",
        &json!({ "bytes": [99, 0, 255, 192, 175, 88] }),
        TIMEOUT,
    );
    let request = started.next_request()?;
    assert_eq!(request.payload, [0, 255, 192, 175]);
    started.runtime.reply(
        request.id,
        tokamak::PluginReply {
            payload: request.payload,
            failed: false,
            done: true,
        },
    );
    assert_eq!(join(event)?, json!([0, 255, 192, 175]));
    Ok(())
}

#[test]
fn streams_a_call_body_as_the_shell_pulls_each_chunk() -> TestResult {
    let started = Started::new(true)?;
    let event = started.emit("upload", &json!({ "chunks": [[0, 255], [7]] }), TIMEOUT);
    let call = started.next_request()?;
    assert_eq!(call.text, "body files.write {}");

    let mut chunks = Vec::new();
    for _ in 0..3 {
        started.runtime.pull(call.id);
        let chunk = started.next_request()?;
        assert_eq!(chunk.id, call.id);
        chunks.push((chunk.text, chunk.payload));
    }
    started.runtime.reply(call.id, value(b"written", true));

    assert_eq!(
        chunks,
        [
            ("chunk more".to_owned(), vec![0, 255]),
            ("chunk more".to_owned(), vec![7]),
            ("chunk done".to_owned(), vec![]),
        ]
    );
    assert_eq!(join(event)?, json!("written"));
    Ok(())
}

#[test]
fn an_aborted_call_cancels_the_shell_work_and_rejects_with_the_reason() -> TestResult {
    let started = Started::new(true)?;
    let event = started.emit("abort", &json!({}), TIMEOUT);
    let call = started.next_request()?;
    let cancel = started.next_request()?;

    assert_eq!(call.text, "call test.hang ");
    assert_eq!((cancel.id, cancel.text.as_str()), (call.id, "cancel"));
    assert_eq!(join(event)?, json!("stopped"));
    Ok(())
}

fn value(payload: &[u8], done: bool) -> tokamak::PluginReply {
    tokamak::PluginReply {
        payload: payload.to_vec(),
        failed: false,
        done,
    }
}
