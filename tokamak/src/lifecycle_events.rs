//! Application lifecycle events.

use std::sync::Arc;

/// Something that happened to the runtime.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Event {
    /// Startup began.
    Starting,
    /// The gateway is accepting connections.
    Listening {
        /// Loopback port the gateway bound.
        port: u16,
    },
    /// JavaScript execution is suspended.
    Suspended,
    /// JavaScript execution resumed.
    Resumed,
    /// Certificates were renewed in the background.
    CertificatesRenewed,
    /// Something failed after the runtime started.
    Failed {
        /// What went wrong.
        message: String,
    },
    /// A request or WebSocket session failed after the gateway accepted it.
    RequestFailed {
        /// What went wrong.
        message: String,
    },
}

/// Delivers events to the shell that started the runtime.
#[derive(Clone)]
pub(crate) struct Events(Arc<dyn Fn(Event) + Send + Sync>);

impl Events {
    pub(crate) fn new(listener: impl Fn(Event) + Send + Sync + 'static) -> Self {
        Self(Arc::new(listener))
    }

    pub(crate) fn emit(&self, event: Event) {
        (self.0)(event);
    }
}

impl std::fmt::Debug for Events {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Events")
    }
}
