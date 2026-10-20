use std::sync::Arc;

use serde_json::{Value, json};
use tao::event_loop::EventLoopProxy;
use tokamak::{Event, PluginHandler, PluginReply, Runtime};
use url::Url;
use wry::WebView;

pub(super) enum ShellEvent {
    Runtime(Event),
    Call {
        id: u64,
        plugin: String,
        method: String,
        payload: Vec<u8>,
    },
    Unsupported(u64),
}

pub(super) struct Controls(pub(super) EventLoopProxy<ShellEvent>);

impl PluginHandler for Controls {
    fn call(&self, id: u64, plugin: &str, method: &str, payload: &[u8]) {
        let _ = self.0.send_event(ShellEvent::Call {
            id,
            plugin: plugin.into(),
            method: method.into(),
            payload: payload.to_vec(),
        });
    }

    fn subscribe(&self, id: u64, _: &str, _: &str, _: &[u8]) {
        let _ = self.0.send_event(ShellEvent::Unsupported(id));
    }

    fn cancel(&self, _: u64) {}
}

pub(super) fn respond(runtime: &Runtime, id: u64, result: Result<Value, (&str, String)>) {
    let (value, failed) = match result {
        Ok(value) => (value, false),
        Err((name, message)) => (json!({ "name": name, "message": message }), true),
    };
    runtime.reply(
        id,
        PluginReply {
            payload: value.to_string().into_bytes(),
            failed,
            done: true,
        },
    );
}

pub(super) fn call(
    view: &WebView,
    runtime: &Arc<Runtime>,
    id: u64,
    plugin: &str,
    method: &str,
    payload: &[u8],
) {
    let result = perform(view, runtime, id, plugin, method, payload);
    if let Some(result) = result.transpose() {
        respond(runtime, id, result);
    }
}

fn perform(
    view: &WebView,
    runtime: &Arc<Runtime>,
    id: u64,
    plugin: &str,
    method: &str,
    payload: &[u8],
) -> Result<Option<Value>, (&'static str, String)> {
    if plugin != "tokamak" || !method.starts_with("webview.") {
        return Err((
            "NotSupportedError",
            format!("{plugin}.{method} is not supported"),
        ));
    }
    let current = view.url().map_err(operation)?;
    let page =
        Url::parse(&current).map_err(|_| ("InvalidStateError", "No page is loaded".into()))?;
    if page.scheme() != "https" {
        return Err(("InvalidStateError", "No app page is loaded".into()));
    }
    let value: Value =
        serde_json::from_slice(payload).map_err(|error| ("DataCloneError", error.to_string()))?;
    match method {
        "webview.getCurrentPage" => Ok(Some(Value::String(current))),
        "webview.navigate" => {
            let url = value
                .as_str()
                .and_then(|path| page.join(path).ok())
                .filter(|url| url.origin() == page.origin())
                .ok_or(("TypeError", "Navigation requires an app URL".into()))?;
            view.load_url(url.as_str()).map_err(operation)?;
            Ok(Some(Value::Null))
        }
        "webview.postMessage" => {
            let runtime = Arc::clone(runtime);
            let data = Value::String(value.to_string());
            let script = format!(
                "(() => {{ window.dispatchEvent(new MessageEvent('message', {{data: JSON.parse({data}), origin: location.origin}})); return true; }})()"
            );
            view.evaluate_script_with_callback(&script, move |result| {
                respond(
                    &runtime,
                    id,
                    if result == "true" {
                        Ok(Value::Null)
                    } else {
                        Err(("OperationError", "Page message delivery failed".into()))
                    },
                );
            })
            .map_err(operation)?;
            Ok(None)
        }
        _ => Err((
            "NotSupportedError",
            format!("{plugin}.{method} is not supported"),
        )),
    }
}

fn operation(error: wry::Error) -> (&'static str, String) {
    ("OperationError", error.to_string())
}
