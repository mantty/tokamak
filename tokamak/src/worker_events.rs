//! Events delivered to the Worker, each named by the handler it calls.

use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use crate::gateway;
use crate::lock;

/// The handler of the event delivered inside the platform's launch.
const BEFORE_START: &str = "_beforeStart";
/// The handler of the event every later event waits for.
const START: &str = "_onStart";
/// How long the Worker has for `_onStart`.
const START_DEADLINE: Duration = Duration::from_secs(10);

/// Delivers events to the Worker: `_beforeStart` at once, `_onStart` when the
/// shell asks, and other events after it, with `_onResume` and `_onSuspend`
/// one at a time.
pub(crate) struct WorkerEvents {
    gateway: gateway::Handle,
    /// Whether `_onStart` has been delivered, or failed.
    started: Mutex<bool>,
    start_finished: Condvar,
    /// Held while a lifecycle event is delivered.
    lifecycle: Mutex<()>,
}

impl WorkerEvents {
    /// The events of the Worker `gateway` serves.
    pub(crate) fn new(gateway: gateway::Handle) -> Arc<Self> {
        Arc::new(Self {
            gateway,
            started: Mutex::new(false),
            start_finished: Condvar::new(),
            lifecycle: Mutex::new(()),
        })
    }

    /// Delivers `_onStart` in the background, with whether the app is in the
    /// `foreground`, and lets other events through once its handler returns.
    pub(crate) fn start(self: &Arc<Self>, foreground: bool) {
        let events = Arc::clone(self);
        let spawned = thread::Builder::new()
            .name("tokamak-start".to_owned())
            .spawn(move || {
                let event = format!(r#"{{"foreground":{foreground}}}"#);
                let deadline = Instant::now() + START_DEADLINE;
                let delivered = events.gateway.deliver(START, event.as_bytes(), deadline);
                events.finish_start(delivered.err());
            });
        if let Err(error) = spawned {
            self.finish_start(Some(error.to_string()));
        }
    }

    /// Delivers the event `name`, whose encoded data is `event`, and returns
    /// the handler's encoded reply, which is empty when it replies nothing.
    /// Every event but `_beforeStart` waits for `_onStart`.
    pub(crate) fn emit(
        &self,
        name: &str,
        event: &[u8],
        timeout: Duration,
    ) -> Result<Vec<u8>, String> {
        let deadline = Instant::now() + timeout;
        if [START, ""].contains(&name) {
            return Err(format!("{name:?} cannot be emitted"));
        }
        if name != BEFORE_START {
            self.wait_for_start(name, deadline)?;
        }
        let lifecycle = matches!(name, "_onResume" | "_onSuspend");
        let _turn = lifecycle.then(|| lock(&self.lifecycle));
        self.gateway.deliver(name, event, deadline)
    }

    /// Lets events through after `_onStart`, reporting why it failed.
    fn finish_start(&self, failure: Option<String>) {
        if let Some(message) = failure {
            self.gateway
                .report_failure(format!("{START} failed: {message}"));
        }
        *lock(&self.started) = true;
        self.start_finished.notify_all();
    }

    fn wait_for_start(&self, name: &str, deadline: Instant) -> Result<(), String> {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let (_started, waited) = self
            .start_finished
            .wait_timeout_while(lock(&self.started), remaining, |started| !*started)
            .unwrap_or_else(PoisonError::into_inner);
        if waited.timed_out() {
            return Err(format!("{name} timed out waiting for {START}"));
        }
        Ok(())
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
