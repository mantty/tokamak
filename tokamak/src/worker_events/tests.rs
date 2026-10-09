use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::json;

use super::WorkerEvents;
use crate::dispatcher::Dispatcher;
use crate::gateway;
use crate::lifecycle_events::{Event, Events};
use crate::quickjs::WorkerBundle;
use crate::tests::{certificates, runtime_config};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Logs each event to `env.LOG` as it begins; waits `event.wait`, or
/// `env.START_DELAY` for `start`, then logs that it is done; and lingers in
/// `waitUntil` for `event.linger`.
const EVENT_WORKER: &[u8] = br#"
import { WorkerEntrypoint } from "cloudflare:workers";

const sleep = (milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds));

export class TokamakEvents extends WorkerEntrypoint {
  async dispatch(name, event) {
    const log = (entry) => fetch(`${this.env.LOG}/?entry=${encodeURIComponent(entry)}`);
    await log(`${name} ${JSON.stringify(event)}`);
    if (name === "broken") throw new Error("listener exploded");
    const wait = name === "start" ? this.env.START_DELAY : event.wait;
    if (wait) {
      await sleep(wait);
      await log(`${name} done`);
    }
    if (event.linger) this.ctx.waitUntil(sleep(event.linger).then(() => log(`${name} lingered`)));
    return { reply: { name, event }, listened: ["start", "resume", "suspend", "echo", "broken"] };
  }
}

export default { fetch: () => new Response(null, { status: 204 }) };
"#;

/// Events delivered to a Worker, and what it logged.
struct Started {
    events: Arc<WorkerEvents>,
    log: flume::Receiver<String>,
    failures: flume::Receiver<Event>,
    _gateway: gateway::Runtime,
    _directory: tempfile::TempDir,
}

impl Started {
    /// `source`'s events, with `start` delivered with `foreground` and taking
    /// `start_delay` milliseconds.
    fn new(source: &[u8], foreground: bool, start_delay: u64) -> TestResult<Self> {
        let (url, log) = log_server()?;
        let directory = tempfile::tempdir()?;
        let worker = WorkerBundle::of_source(source, directory.path())?;
        let mut config = runtime_config();
        config.environment = BTreeMap::from([
            ("LOG".to_owned(), json!(url)),
            ("START_DELAY".to_owned(), json!(start_delay)),
        ]);
        let (sink, failures) = flume::unbounded();
        let gateway = gateway::Runtime::start(
            Dispatcher::new(worker, config),
            certificates(directory.path())?,
            "example.test".to_owned(),
            Events::new(move |event| {
                if matches!(event, Event::Failed { .. }) {
                    let _ = sink.send(event);
                }
            }),
        )?;
        let events = WorkerEvents::start(Arc::clone(gateway.shared()), foreground);
        Ok(Self {
            events,
            log,
            failures,
            _gateway: gateway,
            _directory: directory,
        })
    }

    fn emit(&self, name: &str, event: &str) -> Result<String, String> {
        self.events.emit(name, event, Duration::from_secs(5))
    }

    /// Everything logged so far.
    fn logged(&self) -> Vec<String> {
        self.log.drain().collect()
    }
}

/// A URL whose requests' `entry` parameters arrive on the returned receiver.
fn log_server() -> TestResult<(String, flume::Receiver<String>)> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let url = format!("http://127.0.0.1:{}", listener.local_addr()?.port());
    let (sender, log) = flume::unbounded();
    thread::spawn(move || -> std::io::Result<()> {
        for stream in listener.incoming() {
            let mut stream = stream?;
            let mut line = String::new();
            BufReader::new(&stream).read_line(&mut line)?;
            let query = line.split(['?', ' ']).nth(2).unwrap_or_default();
            for (_, entry) in url::form_urlencoded::parse(query.as_bytes()) {
                let _ = sender.send(entry.into_owned());
            }
            stream.write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n")?;
        }
        Ok(())
    });
    Ok((url, log))
}

#[test]
fn delivers_start_first_and_queues_events_behind_it() -> TestResult {
    let started = Started::new(EVENT_WORKER, false, 200)?;

    let reply = started.emit("echo", r#"{"n":1}"#)?;

    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&reply)?,
        json!({ "name": "echo", "event": { "n": 1 } })
    );
    assert_eq!(
        started.logged(),
        [
            r#"start {"foreground":false}"#,
            "start done",
            r#"echo {"n":1}"#
        ]
    );
    Ok(())
}

#[test]
fn starts_no_worker_for_an_event_without_listeners() -> TestResult {
    let started = Started::new(EVENT_WORKER, true, 0)?;

    assert_eq!(started.emit("unlistened", "{}")?, "null");
    started.emit("echo", "{}")?;

    assert_eq!(
        started.logged(),
        [r#"start {"foreground":true}"#, "echo {}"]
    );
    Ok(())
}

#[test]
fn delivers_resume_and_suspend_only_when_the_foreground_changes() -> TestResult {
    let started = Started::new(EVENT_WORKER, true, 0)?;

    for name in ["resume", "suspend", "suspend", "resume"] {
        started.emit(name, "{}")?;
    }

    assert_eq!(
        started.logged(),
        [r#"start {"foreground":true}"#, "suspend {}", "resume {}"]
    );
    Ok(())
}

#[test]
fn delivers_lifecycle_events_one_at_a_time() -> TestResult {
    let started = Arc::new(Started::new(EVENT_WORKER, false, 0)?);
    started.emit("echo", "{}")?;
    let resuming = Arc::clone(&started);
    let resume = thread::spawn(move || resuming.emit("resume", r#"{"wait":300}"#));
    let mut logged = Vec::new();
    while logged.last().map(String::as_str) != Some(r#"resume {"wait":300}"#) {
        logged.push(started.log.recv_timeout(Duration::from_secs(5))?);
    }

    started.emit("suspend", "{}")?;
    resume.join().map_err(|_| "resume panicked")??;

    logged.extend(started.logged());
    assert_eq!(
        logged,
        [
            r#"start {"foreground":false}"#,
            "echo {}",
            r#"resume {"wait":300}"#,
            "resume done",
            "suspend {}"
        ]
    );
    Ok(())
}

#[test]
fn completes_once_wait_until_promises_settle() -> TestResult {
    let started = Started::new(EVENT_WORKER, true, 0)?;

    started.emit("echo", r#"{"linger":100}"#)?;

    assert_eq!(
        started.logged(),
        [
            r#"start {"foreground":true}"#,
            r#"echo {"linger":100}"#,
            "echo lingered"
        ]
    );
    Ok(())
}

#[test]
fn replies_at_the_deadline_while_wait_until_promises_are_pending() -> TestResult {
    let started = Started::new(EVENT_WORKER, true, 0)?;
    started.emit("echo", "{}")?;
    let begun = Instant::now();

    let reply = started
        .events
        .emit("echo", r#"{"linger":60000}"#, Duration::from_secs(2))?;

    assert!(reply.contains(r#""linger":60000"#), "{reply}");
    assert!(begun.elapsed() < Duration::from_secs(10));
    Ok(())
}

#[test]
fn fails_an_event_whose_listeners_outlast_the_deadline() -> TestResult {
    let started = Started::new(EVENT_WORKER, true, 0)?;
    started.emit("echo", "{}")?;
    let begun = Instant::now();

    let error = started
        .events
        .emit("echo", r#"{"wait":3000}"#, Duration::from_millis(300))
        .err()
        .ok_or("the event completed")?;

    assert_eq!(error, "echo did not complete before its deadline");
    assert!(begun.elapsed() < Duration::from_secs(2));
    Ok(())
}

#[test]
fn fails_an_event_the_worker_throws_in() -> TestResult {
    let started = Started::new(EVENT_WORKER, true, 0)?;

    let error = started
        .emit("broken", "{}")
        .err()
        .ok_or("the event completed")?;

    assert!(error.contains("listener exploded"), "{error}");
    Ok(())
}

#[test]
fn refuses_to_emit_start() -> TestResult {
    let started = Started::new(EVENT_WORKER, true, 0)?;

    assert!(started.emit("start", "{}").is_err());
    assert!(started.emit("", "{}").is_err());
    Ok(())
}

#[test]
fn reports_a_failed_start_and_still_delivers_events() -> TestResult {
    let started = Started::new(
        b"export default { fetch: () => new Response(null) };",
        true,
        0,
    )?;

    let error = started
        .emit("echo", "{}")
        .err()
        .ok_or("the event completed")?;

    assert!(error.contains("does not export TokamakEvents"), "{error}");
    let failure = started.failures.recv_timeout(Duration::from_secs(5))?;
    assert!(
        matches!(&failure, Event::Failed { message } if message.starts_with("start failed: ")),
        "{failure:?}"
    );
    Ok(())
}
