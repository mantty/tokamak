//! Events delivered to the Worker's listeners.

use std::collections::BTreeSet;
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use crate::gateway;
use crate::lock;

/// How long the Worker has for `start`.
const START_DEADLINE: Duration = Duration::from_secs(10);

/// Delivers events to the Worker: `start` first, then other events, with
/// lifecycle events one at a time. A packaged Worker reports the events it
/// listens for, and no Worker runs for any other.
pub(crate) struct WorkerEvents {
    gateway: gateway::Handle,
    state: Mutex<State>,
    started: Condvar,
    /// Held while a lifecycle event is delivered.
    lifecycle: Mutex<()>,
}

struct State {
    /// Whether `start` has been delivered, or failed.
    started: bool,
    /// The events that have listeners, once the Worker has reported them.
    listened: Option<BTreeSet<String>>,
}

impl WorkerEvents {
    /// Delivers `start` to the Worker `gateway` serves, in the background.
    pub(crate) fn start(gateway: gateway::Handle, foreground: bool) -> Arc<Self> {
        let events = Arc::new(Self {
            gateway,
            state: Mutex::new(State {
                started: false,
                listened: None,
            }),
            started: Condvar::new(),
            lifecycle: Mutex::new(()),
        });
        let starting = Arc::clone(&events);
        let spawned = thread::Builder::new()
            .name("tokamak-start".to_owned())
            .spawn(move || starting.deliver_start(foreground));
        if let Err(error) = spawned {
            events.finish_start(Some(format!("start could not be delivered: {error}")));
        }
        events
    }

    /// Delivers the event `name`, whose encoded data is `event`, once `start`
    /// has been, and returns the encoded reply, which is empty when no
    /// listener replied.
    pub(crate) fn emit(
        &self,
        name: &str,
        event: &[u8],
        timeout: Duration,
    ) -> Result<Vec<u8>, String> {
        let deadline = Instant::now() + timeout;
        if name.is_empty() || name == "start" {
            return Err(format!("{name:?} cannot be emitted"));
        }
        self.wait_for_start(name, deadline)?;
        let lifecycle = matches!(name, "resume" | "suspend");
        let _turn = lifecycle.then(|| lock(&self.lifecycle));
        if !self.listens(name) {
            return Ok(Vec::new());
        }
        self.deliver(name, event, deadline)
    }

    fn deliver_start(&self, foreground: bool) {
        let event = format!(r#"{{"foreground":{foreground}}}"#);
        let delivered = self.deliver("start", event.as_bytes(), Instant::now() + START_DEADLINE);
        self.finish_start(
            delivered
                .err()
                .map(|message| format!("start failed: {message}")),
        );
    }

    /// Lets events through after `start`, reporting its `failure` unless the
    /// runtime is stopping.
    fn finish_start(&self, failure: Option<String>) {
        if let Some(message) = failure {
            self.gateway.report_failure(message);
        }
        lock(&self.state).started = true;
        self.started.notify_all();
    }

    fn wait_for_start(&self, name: &str, deadline: Instant) -> Result<(), String> {
        let mut state = lock(&self.state);
        while !state.started {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(format!("{name} timed out waiting for start"));
            }
            state = self
                .started
                .wait_timeout(state, remaining)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        Ok(())
    }

    /// Whether the event `name` may have listeners.
    fn listens(&self, name: &str) -> bool {
        lock(&self.state)
            .listened
            .as_ref()
            .is_none_or(|listened| listened.contains(name))
    }

    fn deliver(&self, name: &str, event: &[u8], deadline: Instant) -> Result<Vec<u8>, String> {
        let delivery = self.gateway.deliver(name, event, deadline)?;
        if let Some(listened) = delivery.listened {
            lock(&self.state).listened = Some(listened);
        }
        Ok(delivery.reply)
    }
}

impl std::fmt::Debug for WorkerEvents {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WorkerEvents")
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests;
