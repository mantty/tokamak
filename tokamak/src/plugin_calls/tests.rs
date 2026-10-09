use std::sync::{Arc, Mutex, PoisonError};

use serde_json::{Value, json};

use super::{PluginCalls, PluginHandler};

/// A shell that records what the runtime asks of it.
#[derive(Default)]
struct Shell {
    requests: Mutex<Vec<String>>,
}

impl Shell {
    fn record(&self, request: String) {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
    }

    fn requests(&self) -> Vec<String> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl PluginHandler for Shell {
    fn call(&self, id: u64, plugin: &str, method: &str, arguments: &str) {
        self.record(format!("call {id} {plugin}.{method} {arguments}"));
    }

    fn subscribe(&self, id: u64, plugin: &str, method: &str, arguments: &str) {
        self.record(format!("subscribe {id} {plugin}.{method} {arguments}"));
    }

    fn unsubscribe(&self, id: u64) {
        self.record(format!("unsubscribe {id}"));
    }
}

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn with_shell() -> (Arc<Shell>, Arc<PluginCalls>) {
    let shell = Arc::new(Shell::default());
    let calls = PluginCalls::new(Some(Arc::clone(&shell) as Arc<dyn PluginHandler>), true);
    (shell, calls)
}

fn parse(result: &str) -> Result<Value, serde_json::Error> {
    serde_json::from_str(result)
}

#[tokio::test]
async fn resolves_a_call_with_the_shell_reply() -> TestResult {
    let (shell, calls) = with_shell();

    let result = calls.call("location", "getCurrentPosition", r#"{"timeout":5}"#);
    calls.reply(1, r#"{"value":{"latitude":51.5},"done":true}"#);
    calls.reply(1, r#"{"value":"again","done":true}"#);

    assert_eq!(
        parse(&result.await)?,
        json!({ "value": { "latitude": 51.5 }, "done": true })
    );
    assert_eq!(
        shell.requests(),
        [r#"call 1 location.getCurrentPosition {"timeout":5}"#]
    );
    Ok(())
}

#[tokio::test]
async fn delivers_each_subscription_result_until_one_is_done() -> TestResult {
    let (shell, calls) = with_shell();

    let mut subscription = calls.subscribe("location", "watchPosition", "null");
    calls.reply(1, r#"{"value":1,"done":false}"#);
    calls.reply(
        1,
        r#"{"error":{"name":"NotReadableError","message":"no fix"},"done":false}"#,
    );
    calls.reply(1, r#"{"value":2,"done":true}"#);
    calls.reply(1, r#"{"value":3,"done":false}"#);

    let mut results = Vec::new();
    while let Some(result) = subscription.next().await {
        results.push(parse(&result)?);
    }
    drop(subscription);

    assert_eq!(
        results,
        [
            json!({ "value": 1, "done": false }),
            json!({ "error": { "name": "NotReadableError", "message": "no fix" }, "done": false }),
            json!({ "value": 2, "done": true }),
        ]
    );
    assert_eq!(
        shell.requests(),
        ["subscribe 1 location.watchPosition null"]
    );
    Ok(())
}

#[test]
fn unsubscribes_at_the_shell_when_a_subscription_is_dropped() {
    let (shell, calls) = with_shell();

    drop(calls.subscribe("location", "watchPosition", "null"));
    calls.reply(1, r#"{"value":1,"done":false}"#);

    assert_eq!(
        shell.requests(),
        ["subscribe 1 location.watchPosition null", "unsubscribe 1"]
    );
}

#[test]
fn ignores_the_result_of_a_call_no_longer_waited_for() {
    let (shell, calls) = with_shell();

    drop(calls.call("location", "getCurrentPosition", "null"));
    calls.reply(1, r#"{"value":1,"done":true}"#);
    calls.reply(7, r#"{"value":1,"done":true}"#);

    assert_eq!(
        shell.requests(),
        ["call 1 location.getCurrentPosition null"]
    );
}

#[tokio::test]
async fn replaces_a_result_that_is_not_json_with_an_error() -> TestResult {
    let (shell, calls) = with_shell();

    let result = calls.call("location", "getCurrentPosition", "null");
    let mut subscription = calls.subscribe("location", "watchPosition", "null");
    calls.reply(1, "not json");
    calls.reply(2, "not json");
    calls.reply(2, r#"{"value":1,"done":false}"#);

    assert_eq!(parse(&result.await)?["error"]["name"], "OperationError");
    let failure = parse(&subscription.next().await.ok_or("no result")?)?;
    assert_eq!(failure["error"]["name"], "OperationError");
    assert_eq!(
        parse(&subscription.next().await.ok_or("no result")?)?["value"],
        1
    );
    drop(subscription);
    assert_eq!(
        shell.requests().last().map(String::as_str),
        Some("unsubscribe 2")
    );
    Ok(())
}

#[tokio::test]
async fn answers_the_lifecycle_stage_itself() -> TestResult {
    let (shell, calls) = with_shell();

    assert_eq!(
        parse(&calls.call("tokamak", "lifecycleStage", "null").await)?,
        json!({ "value": "foreground", "done": true })
    );
    assert!(calls.set_foreground(false));
    assert!(!calls.set_foreground(false));
    assert_eq!(
        parse(&calls.call("tokamak", "lifecycleStage", "null").await)?["value"],
        "background"
    );
    assert_eq!(
        parse(&calls.call("tokamak", "unknown", "null").await)?["error"]["name"],
        "NotSupportedError"
    );
    assert!(shell.requests().is_empty());
    Ok(())
}

#[tokio::test]
async fn fails_plugin_calls_without_a_shell_handler() -> TestResult {
    let calls = PluginCalls::new(None, false);

    let result = parse(&calls.call("location", "getCurrentPosition", "null").await)?;
    let mut subscription = calls.subscribe("location", "watchPosition", "null");

    assert_eq!(result["error"]["name"], "NotSupportedError");
    assert_eq!(
        result["error"]["message"],
        "location.getCurrentPosition is not supported"
    );
    let first = subscription.next().await.ok_or("no result")?;
    assert_eq!(parse(&first)?["error"]["name"], "NotSupportedError");
    assert_eq!(
        parse(&calls.call("tokamak", "lifecycleStage", "null").await)?["value"],
        "background"
    );
    Ok(())
}
