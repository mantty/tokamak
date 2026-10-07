//! Application lifecycle events.

use std::fmt;
use std::sync::Arc;

/// Something that happened to the runtime.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Event {
    /// Startup began.
    Starting,
    /// The gateway is accepting connections, on a new port when its
    /// listener was replaced.
    Listening {
        /// Loopback port the gateway bound.
        port: u16,
    },
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

impl fmt::Display for Event {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Starting => formatter.write_str("runtime starting"),
            Self::Listening { port } => write!(formatter, "gateway listening on 127.0.0.1:{port}"),
            Self::CertificatesRenewed => formatter.write_str("certificates renewed"),
            Self::Failed { message } => write!(formatter, "runtime failed: {message}"),
            Self::RequestFailed { message } => write!(formatter, "request failed: {message}"),
        }
    }
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

#[cfg(test)]
mod tests {
    use super::Event;

    #[test]
    fn describes_events_for_shell_logs() {
        assert_eq!(
            Event::Listening { port: 8443 }.to_string(),
            "gateway listening on 127.0.0.1:8443"
        );
        assert_eq!(
            Event::RequestFailed {
                message: "chunk size is invalid".to_owned()
            }
            .to_string(),
            "request failed: chunk size is invalid"
        );
    }
}
