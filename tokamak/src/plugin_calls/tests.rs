use std::sync::{Arc, Mutex};

use super::{Overflow, PluginCalls, PluginHandler, PluginReply, SUBSCRIPTION_CAPACITY};
use crate::lock;

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[derive(Default)]
struct Shell {
    requests: Mutex<Vec<(u64, String, Vec<u8>)>>,
    /// The one plugin the shell implements, when it does not implement all.
    only: Option<&'static str>,
}
impl Shell {
    fn record(&self, id: u64, method: String, payload: &[u8]) {
        lock(&self.requests).push((id, method, payload.to_vec()));
    }
    fn requests(&self) -> Vec<(u64, String, Vec<u8>)> {
        lock(&self.requests).clone()
    }
}
impl PluginHandler for Shell {
    fn implements(&self, plugin: &str, _method: &str) -> bool {
        self.only.is_none_or(|only| only == plugin)
    }
    fn call(&self, id: u64, plugin: &str, method: &str, payload: &[u8]) {
        self.record(id, format!("{plugin}.{method}"), payload);
    }
    fn call_with_body(&self, id: u64, plugin: &str, method: &str, payload: &[u8]) {
        self.record(id, format!("{plugin}.{method} with body"), payload);
    }
    fn chunk(&self, id: u64, chunk: &PluginReply) {
        self.record(id, format!("chunk done={}", chunk.done), &chunk.payload);
    }
    fn subscribe(&self, id: u64, plugin: &str, method: &str, payload: &[u8]) {
        self.record(id, format!("{plugin}.{method}"), payload);
    }
    fn cancel(&self, id: u64) {
        self.record(id, "cancel".into(), &[]);
    }
}
fn with_shell() -> (Arc<Shell>, Arc<PluginCalls>) {
    let shell = Arc::new(Shell::default());
    let calls = PluginCalls::new(Some(Arc::clone(&shell) as Arc<dyn PluginHandler>), true);
    (shell, calls)
}
fn value(payload: &[u8], done: bool) -> PluginReply {
    PluginReply {
        payload: payload.to_vec(),
        failed: false,
        done,
    }
}

#[tokio::test]
async fn carries_arbitrary_bytes_without_decoding_and_resolves_once() {
    let (shell, calls) = with_shell();
    let payload = [0, 255, 0xc0, 0xaf];
    let result = calls.call("binary", "echo", &payload);
    calls.reply(1, value(&payload, true));
    calls.reply(1, value(b"late", true));
    let result = result.await;
    assert_eq!(result.payload, payload);
    assert!(!result.failed);
    assert!(result.done);
    assert_eq!(
        shell.requests(),
        [(1, "binary.echo".into(), payload.to_vec())]
    );
}

#[tokio::test]
async fn delivers_values_and_errors_until_explicit_completion() -> TestResult {
    let (shell, calls) = with_shell();
    let mut stream = calls.subscribe("binary", "watch", &[], Overflow::Error);
    calls.reply(1, value(&[255], false));
    calls.reply(
        1,
        PluginReply {
            payload: b"endpoint error".to_vec(),
            failed: true,
            done: false,
        },
    );
    calls.reply(1, value(b"last", true));
    calls.reply(1, value(b"late", false));
    assert_eq!(stream.next().await.ok_or("no value")?.payload, [255]);
    let error = stream.next().await.ok_or("no error")?;
    assert!(error.failed);
    assert!(!error.done);
    assert_eq!(error.payload, b"endpoint error");
    let last = stream.next().await.ok_or("no last value")?;
    assert!(last.done);
    assert_eq!(last.payload, b"last");
    assert!(stream.next().await.is_none());
    drop(stream);
    assert_eq!(shell.requests().len(), 1);
    Ok(())
}

#[tokio::test]
async fn cancelled_requests_release_native_resources_without_affecting_other_calls() {
    let (shell, calls) = with_shell();
    drop(calls.call("a", "call", &[]));
    drop(calls.subscribe("a", "stream", &[], Overflow::Error));
    let retained = calls.call("b", "call", &[]);
    calls.reply(1, value(b"late", true));
    calls.reply(3, value(b"retained", true));
    assert_eq!(retained.await.payload, b"retained");
    assert_eq!(
        shell
            .requests()
            .iter()
            .filter(|(_, method, _)| method == "cancel")
            .map(|(id, _, _)| *id)
            .collect::<Vec<_>>(),
        [1, 2]
    );
}

#[tokio::test]
async fn overflowing_a_subscription_reports_failure_and_cancels_native_work() -> TestResult {
    let (shell, calls) = with_shell();
    let mut stream = calls.subscribe("binary", "watch", &[], Overflow::Error);
    for _ in 0..=SUBSCRIPTION_CAPACITY {
        calls.reply(1, value(&[1], false));
    }
    for _ in 0..SUBSCRIPTION_CAPACITY {
        assert_eq!(
            stream.next().await.ok_or("missing queued value")?.payload,
            [1]
        );
    }
    let error = stream.next().await.ok_or("missing overflow error")?;
    assert!(error.failed && error.done);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&error.payload)?["name"],
        "QuotaExceededError"
    );
    assert!(stream.next().await.is_none());
    drop(stream);
    assert_eq!(shell.requests().last(), Some(&(1, "cancel".into(), vec![])));
    Ok(())
}

#[tokio::test]
async fn runtime_endpoints_encode_their_own_values_and_errors() -> TestResult {
    let (shell, calls) = with_shell();
    assert_eq!(
        calls
            .call("tokamak", "lifecycleStage", b"null")
            .await
            .payload,
        br#""foreground""#
    );
    assert!(calls.set_foreground(false));
    assert!(!calls.set_foreground(false));
    assert_eq!(
        calls
            .call("tokamak", "lifecycleStage", b"null")
            .await
            .payload,
        br#""background""#
    );
    assert!(shell.requests().is_empty());
    let error = PluginCalls::new(None, false)
        .call("tokamak", "unknown", &[])
        .await;
    assert!(error.failed && error.done);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&error.payload)?["name"],
        "NotSupportedError"
    );
    Ok(())
}

#[tokio::test]
async fn missing_native_handler_fails_calls_and_subscriptions() -> TestResult {
    let calls = PluginCalls::new(None, false);
    let result = calls.call("location", "getCurrentPosition", &[]).await;
    assert!(result.failed && result.done);
    let error = serde_json::from_slice::<serde_json::Value>(&result.payload)?;
    assert_eq!(error["name"], "NotSupportedError");
    assert_eq!(
        error["message"],
        "location.getCurrentPosition is not supported"
    );
    let mut stream = calls.subscribe("location", "watch", &[], Overflow::Error);
    assert!(stream.next().await.ok_or("missing error")?.failed);
    assert!(stream.next().await.is_none());
    Ok(())
}

#[tokio::test]
async fn rejects_what_the_shell_does_not_implement_without_passing_it_on() -> TestResult {
    let shell = Arc::new(Shell {
        only: Some("binary"),
        ..Shell::default()
    });
    let calls = PluginCalls::new(Some(Arc::clone(&shell) as Arc<dyn PluginHandler>), true);
    let result = calls.call("location", "getCurrentPosition", &[]).await;
    assert!(result.failed && result.done);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&result.payload)?["message"],
        "location.getCurrentPosition is not supported"
    );
    let mut stream = calls.subscribe("location", "watch", &[], Overflow::Error);
    assert!(stream.next().await.ok_or("missing error")?.failed);
    assert!(stream.next().await.is_none());
    let (result, body) = calls.call_with_body("files", "write", &[]);
    assert!(body.is_none());
    assert!(result.await.failed);
    let echoed = calls.call("binary", "echo", b"implemented");
    calls.reply(1, value(b"implemented", true));
    assert_eq!(echoed.await.payload, b"implemented");
    assert_eq!(
        shell.requests(),
        [(1, "binary.echo".into(), b"implemented".to_vec())]
    );
    Ok(())
}

#[tokio::test]
async fn a_drop_oldest_subscription_keeps_the_newest_results() -> TestResult {
    let (shell, calls) = with_shell();
    let mut stream = calls.subscribe("sensor", "watch", &[], Overflow::DropOldest);
    for value in 0..=SUBSCRIPTION_CAPACITY {
        calls.reply(1, value_of(value));
    }
    for value in 1..=SUBSCRIPTION_CAPACITY {
        assert_eq!(
            stream.next().await.ok_or("missing value")?.payload,
            value_of(value).payload
        );
    }
    calls.reply(1, value(b"last", true));
    assert_eq!(
        stream.next().await.ok_or("missing last value")?.payload,
        b"last"
    );
    assert!(stream.next().await.is_none());
    assert!(
        shell
            .requests()
            .iter()
            .all(|(_, method, _)| method != "cancel")
    );
    Ok(())
}

#[tokio::test]
async fn a_call_body_sends_each_chunk_the_shell_pulls_until_the_call_ends() -> TestResult {
    let (shell, calls) = with_shell();
    let (result, body) = calls.call_with_body("files", "write", b"args");
    let mut body = body.ok_or("the shell runs the call")?;
    calls.pull(1);
    assert!(body.pulled().await);
    body.send(&value(&[0, 255], false));
    calls.pull(1);
    assert!(body.pulled().await);
    body.send(&value(&[], true));
    calls.reply(1, value(b"written", true));
    assert_eq!(result.await.payload, b"written");
    calls.pull(1);
    assert!(!body.pulled().await, "an ended call still pulled");
    assert_eq!(
        shell.requests(),
        [
            (1, "files.write with body".into(), b"args".to_vec()),
            (1, "chunk done=false".into(), vec![0, 255]),
            (1, "chunk done=true".into(), vec![]),
        ]
    );
    Ok(())
}

#[tokio::test]
async fn cancelling_a_call_with_a_body_ends_its_body() -> TestResult {
    let (shell, calls) = with_shell();
    let (result, body) = calls.call_with_body("files", "write", &[]);
    drop(result);
    let mut body = body.ok_or("the shell runs the call")?;
    assert!(!body.pulled().await);
    assert_eq!(shell.requests().last(), Some(&(1, "cancel".into(), vec![])));
    Ok(())
}

fn value_of(value: usize) -> PluginReply {
    super::PluginReply {
        payload: value.to_le_bytes().to_vec(),
        failed: false,
        done: false,
    }
}
