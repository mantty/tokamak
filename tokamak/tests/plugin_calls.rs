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
    Config, ModuleType, PackageLayout, PluginHandler, Runtime, WorkerEnvironment, WorkerManifest,
    write_worker, write_worker_environment,
};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Runs the method of `TokamakEvents` each event names.
const WORKER: &[u8] = br#"
import { WorkerEntrypoint } from "cloudflare:workers";

const call = (plugin, method, args) => globalThis.__tokamakNativeCall(plugin, method, args);
const failure = (error) => ({ name: error.name, message: error.message, domException: error instanceof DOMException });
const sleep = (milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds));

export class TokamakEvents extends WorkerEntrypoint {
  async dispatch(name, event) {
    const reply = name === "start" || name === "suspend" ? undefined : await this[name](event);
    return { reply, listened: ["start", "resume", "suspend", "call", "stage", "watch", "forget"] };
  }

  async resume(event) {
    void call("test", "resuming");
    await sleep(event.wait);
  }

  call(event) {
    return call(event.plugin, event.method, event.arguments).catch(failure);
  }

  stage() {
    return call("tokamak", "lifecycleStage");
  }

  // Reports each position with `test.received`, discarding the function that
  // removes the listener.
  forget() {
    globalThis.__tokamakNativeListen("location", "watchPosition", null, (position) => {
      void call("test", "received", position);
    }, () => undefined);
  }

  // Reports each position with `test.received` until one is "stop", then
  // with `test.stopped` in `waitUntil`, and with `test.alive` if the context
  // still runs half a second later.
  watch() {
    const stop = globalThis.__tokamakNativeListen("location", "watchPosition", null, (position) => {
      if (position !== "stop") return void call("test", "received", position);
      stop();
      this.ctx.waitUntil(call("test", "stopped"));
      setTimeout(() => void call("test", "alive"), 500);
    }, (error) => void call("test", "failed", failure(error)));
  }
}

export default { fetch: () => new Response(null, { status: 204 }) };
"#;

const TIMEOUT: Duration = Duration::from_secs(5);

/// A request the runtime made of the shell's plugins.
#[derive(Debug)]
struct Request {
    id: u64,
    /// The operation, plugin, method and arguments, or `unsubscribe`.
    text: String,
}

/// Shell plugins whose requests the test answers.
struct Shell(Sender<Request>);

impl Shell {
    fn record(&self, id: u64, text: String) {
        let _ = self.0.send(Request { id, text });
    }
}

impl PluginHandler for Shell {
    fn call(&self, id: u64, plugin: &str, method: &str, arguments: &str) {
        self.record(id, format!("call {plugin}.{method} {arguments}"));
    }

    fn subscribe(&self, id: u64, plugin: &str, method: &str, arguments: &str) {
        self.record(id, format!("subscribe {plugin}.{method} {arguments}"));
    }

    fn unsubscribe(&self, id: u64) {
        self.record(id, "unsubscribe".to_owned());
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
        fs::create_dir_all(app.root())?;
        write_worker_environment(&app, &WorkerEnvironment::default())?;
        fs::write(directory.path().join("worker.mjs"), WORKER)?;
        write_worker(
            &app,
            directory.path(),
            &WorkerManifest {
                entry: "worker.mjs".to_owned(),
                modules: BTreeMap::from([("worker.mjs".to_owned(), ModuleType::EsModule)]),
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

    /// Delivers the event `name` on another thread, returning its reply as
    /// JSON once it completes within `timeout`.
    fn emit(&self, name: &str, event: &Value, timeout: Duration) -> JoinHandle<TestResult<Value>> {
        let runtime = Arc::clone(&self.runtime);
        let (name, event) = (name.to_owned(), event.to_string());
        thread::spawn(move || {
            Ok(serde_json::from_str(
                &runtime.emit(&name, &event, timeout)?,
            )?)
        })
    }

    /// The reply to the event `name`.
    fn reply_to(&self, name: &str, event: &Value) -> TestResult<Value> {
        join(self.emit(name, event, TIMEOUT))
    }

    fn next_request(&self) -> TestResult<Request> {
        Ok(self.requests.recv_timeout(TIMEOUT)?)
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
        .reply(request.id, r#"{"value":{"latitude":51.5},"done":true}"#);

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
        r#"{"error":{"name":"NeedsUIError","message":"The app is in the background"},"done":true}"#,
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
fn keeps_listening_past_the_deadline_until_the_last_listener_is_removed() -> TestResult {
    let started = Started::new(true)?;
    join(started.emit("watch", &json!({}), Duration::from_millis(300)))?;
    let subscription = started.next_request()?;
    assert_eq!(subscription.text, "subscribe location.watchPosition null");
    thread::sleep(Duration::from_millis(500));

    let deliver = |result: &str| started.runtime.reply(subscription.id, result);
    deliver(r#"{"value":1,"done":false}"#);
    assert_eq!(started.next_request()?.text, "call test.received 1");
    deliver(r#"{"error":{"name":"NotReadableError","message":"No fix"},"done":false}"#);
    assert_eq!(
        started.next_request()?.text,
        r#"call test.failed {"name":"NotReadableError","message":"No fix","domException":true}"#
    );
    deliver(r#"{"value":"stop","done":false}"#);

    let mut requests = [started.next_request()?, started.next_request()?];
    requests.sort_by_key(|request| request.text.clone());
    let [stopped, unsubscribed] = requests;
    assert_eq!(stopped.text, "call test.stopped null");
    assert_eq!(
        (unsubscribed.id, unsubscribed.text.as_str()),
        (subscription.id, "unsubscribe")
    );
    started
        .runtime
        .reply(stopped.id, r#"{"value":null,"done":true}"#);
    let alive = started.requests.recv_timeout(Duration::from_secs(1));
    assert!(alive.is_err(), "the context still ran: {alive:?}");
    Ok(())
}

#[test]
fn keeps_a_listener_whose_remove_function_is_discarded() -> TestResult {
    let started = Started::new(true)?;
    join(started.emit("forget", &json!({}), TIMEOUT))?;
    let subscription = started.next_request()?;

    started
        .runtime
        .reply(subscription.id, r#"{"value":1,"done":false}"#);

    assert_eq!(started.next_request()?.text, "call test.received 1");
    Ok(())
}

#[test]
fn ends_a_listener_whose_result_is_done() -> TestResult {
    let started = Started::new(true)?;
    join(started.emit("watch", &json!({}), TIMEOUT))?;
    let subscription = started.next_request()?;

    started.runtime.reply(
        subscription.id,
        r#"{"error":{"name":"NotSupportedError","message":"Unsupported"},"done":true}"#,
    );
    assert_eq!(
        started.next_request()?.text,
        r#"call test.failed {"name":"NotSupportedError","message":"Unsupported","domException":true}"#
    );
    started
        .runtime
        .reply(subscription.id, r#"{"value":1,"done":false}"#);

    let after = started.requests.recv_timeout(Duration::from_secs(1));
    assert!(after.is_err(), "the listener still ran: {after:?}");
    Ok(())
}

#[test]
fn unsubscribes_listeners_when_the_runtime_stops() -> TestResult {
    let started = Started::new(true)?;
    join(started.emit("watch", &json!({}), TIMEOUT))?;
    let subscription = started.next_request()?;

    let Started {
        runtime, requests, ..
    } = started;
    drop(Arc::into_inner(runtime).ok_or("the runtime is still shared")?);

    let unsubscribed = requests.recv_timeout(TIMEOUT)?;
    assert_eq!(
        (unsubscribed.id, unsubscribed.text.as_str()),
        (subscription.id, "unsubscribe")
    );
    Ok(())
}
