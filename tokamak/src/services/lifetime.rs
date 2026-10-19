use super::Streams;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

/// The caller's work window and the host's independent shutdown signal.
#[derive(Clone)]
pub(crate) struct Lifetime {
    pub(crate) caller: CancellationToken,
    pub(crate) stopped: CancellationToken,
    pub(crate) deadline: Option<Instant>,
}

impl Lifetime {
    pub(crate) fn background_deadline(&self) -> Instant {
        let maximum = Instant::now() + Duration::from_secs(30);
        self.deadline
            .map_or(maximum, |deadline| deadline.min(maximum))
    }

    pub(crate) fn expired(&self) -> bool {
        self.stopped.is_cancelled()
            || self
                .deadline
                .is_some_and(|deadline| Instant::now() >= deadline)
    }

    pub(crate) async fn cancelled(&self) {
        tokio::select! {
            () = self.caller.cancelled() => {},
            () = self.terminated() => {},
        }
    }

    pub(crate) async fn terminated(&self) {
        let expiry = async {
            match self.deadline {
                Some(deadline) => tokio::time::sleep_until(deadline.into()).await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            () = self.stopped.cancelled() => {},
            () = expiry => {},
        }
    }
}

/// Outbound calls belong to their invoking worker, including its waitUntil work.
pub(crate) struct Scope {
    pub(crate) lifetime: Lifetime,
    pub(crate) streams: Arc<Streams>,
}

impl Scope {
    pub(crate) fn new(stopped: &CancellationToken) -> Self {
        Self {
            lifetime: Lifetime {
                caller: stopped.child_token(),
                stopped: stopped.clone(),
                deadline: None,
            },
            streams: Arc::new(Streams::default()),
        }
    }

    pub(crate) fn within(lifetime: &Lifetime) -> Self {
        Self {
            lifetime: Lifetime {
                caller: lifetime.stopped.child_token(),
                ..lifetime.clone()
            },
            streams: Arc::new(Streams::default()),
        }
    }

    pub(crate) fn until(stopped: &CancellationToken, deadline: Instant) -> Self {
        let mut scope = Self::new(stopped);
        scope.lifetime.deadline = Some(deadline);
        scope
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        self.streams.cancel();
        self.lifetime.caller.cancel();
    }
}
