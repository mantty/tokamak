//! Events delivered to the Worker's listeners.

use std::collections::BTreeSet;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use crate::gateway::{self, Shared};
use crate::lifecycle_events::Event;

/// How long the Worker has for `start`.
const START_DEADLINE: Duration = Duration::from_secs(10);

/// The reply of an event no listener answered.
const NO_REPLY: &str = "null";

/// Delivers events to the Worker: `start` first, then other events, with
/// lifecycle events one at a time. A packaged Worker reports the events it
/// listens for, and no Worker runs for any other.
pub(crate) struct WorkerEvents {
    shared: Arc<Shared>,
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
    /// Delivers `start` to the Worker `shared` serves, in the background.
    pub(crate) fn start(shared: Arc<Shared>, foreground: bool) -> Arc<Self> {
        let events = Arc::new(Self {
            shared,
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

    /// Delivers the event `name`, whose JSON is `event`, once `start` has
    /// been, and returns the reply as JSON.
    pub(crate) fn emit(
        &self,
        name: &str,
        event: &str,
        timeout: Duration,
    ) -> Result<String, String> {
        let deadline = Instant::now() + timeout;
        if name.is_empty() || name == "start" {
            return Err(format!("{name:?} cannot be emitted"));
        }
        self.wait_for_start(name, deadline)?;
        let lifecycle = matches!(name, "resume" | "suspend");
        let _turn = lifecycle.then(|| lock(&self.lifecycle));
        if !self.listens(name) {
            return Ok(NO_REPLY.to_owned());
        }
        self.deliver(name, event, deadline)
    }

    fn deliver_start(&self, foreground: bool) {
        let event = format!(r#"{{"foreground":{foreground}}}"#);
        let delivered = self.deliver("start", &event, Instant::now() + START_DEADLINE);
        self.finish_start(
            delivered
                .err()
                .map(|message| format!("start failed: {message}")),
        );
    }

    /// Lets events through after `start`, reporting its `failure` unless the
    /// runtime is stopping.
    fn finish_start(&self, failure: Option<String>) {
        if let Some(message) = failure
            && !self.shared.stopped.is_cancelled()
        {
            self.shared.events.emit(Event::Failed { message });
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

    fn deliver(&self, name: &str, event: &str, deadline: Instant) -> Result<String, String> {
        let delivery = gateway::deliver(&self.shared, name, event, deadline)?;
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

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests;
