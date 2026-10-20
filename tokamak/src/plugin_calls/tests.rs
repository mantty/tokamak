use std::sync::{Arc, Mutex, PoisonError};

use super::{PluginCalls, PluginHandler, PluginReply, SUBSCRIPTION_CAPACITY};

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[derive(Default)]
struct Shell {
    requests: Mutex<Vec<(u64, String, Vec<u8>)>>,
}
impl Shell {
    fn record(&self, id: u64, method: String, payload: &[u8]) {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((id, method, payload.to_vec()));
    }
    fn requests(&self) -> Vec<(u64, String, Vec<u8>)> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}
impl PluginHandler for Shell {
    fn call(&self, id: u64, plugin: &str, method: &str, payload: &[u8]) {
        self.record(id, format!("{plugin}.{method}"), payload);
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
    let mut stream = calls.subscribe("binary", "watch", &[]);
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
    drop(calls.subscribe("a", "stream", &[]));
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
    let mut stream = calls.subscribe("binary", "watch", &[]);
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
    let error = calls.call("tokamak", "unknown", &[]).await;
    assert!(error.failed && error.done);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&error.payload)?["name"],
        "NotSupportedError"
    );
    assert!(shell.requests().is_empty());
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
    let mut stream = calls.subscribe("location", "watch", &[]);
    assert!(stream.next().await.ok_or("missing error")?.failed);
    assert!(stream.next().await.is_none());
    Ok(())
}
