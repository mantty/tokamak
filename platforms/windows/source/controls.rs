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

    fn call_with_body(&self, id: u64, _: &str, _: &str, _: &[u8]) {
        let _ = self.0.send_event(ShellEvent::Unsupported(id));
    }

    fn chunk(&self, _: u64, _: &PluginReply) {}

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
    let origin = Url::parse(&format!("https://{}/", runtime.host()))
        .map_err(|error| ("OperationError", error.to_string()))?;
    let page = view
        .url()
        .ok()
        .and_then(|current| Url::parse(&current).ok())
        .filter(|page| page.origin() == origin.origin());
    let value: Value =
        serde_json::from_slice(payload).map_err(|error| ("DataCloneError", error.to_string()))?;
    match method {
        "webview.getCurrentPage" => Ok(Some(
            page.map_or(Value::Null, |page| Value::String(page.into())),
        )),
        "webview.navigate" => {
            let url = value
                .as_str()
                .and_then(|path| page.as_ref().unwrap_or(&origin).join(path).ok())
                .filter(|url| url.origin() == origin.origin())
                .ok_or(("TypeError", "Navigation requires an app URL".into()))?;
            view.load_url(url.as_str())
                .map_err(|error| ("OperationError", error.to_string()))?;
            Ok(Some(Value::Null))
        }
        "webview.postMessage" => {
            if page.is_none() {
                return Err(("InvalidStateError", "No page is loaded".into()));
            }
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
            .map_err(|error| ("OperationError", error.to_string()))?;
            Ok(None)
        }
        _ => Err((
            "NotSupportedError",
            format!("{plugin}.{method} is not supported"),
        )),
    }
}
