//! Blocking readiness waits on a gateway socket, interruptible from other threads.

use std::io;
use std::net::TcpStream;
use std::time::Instant;

use mio::{Events, Interest, Poll, Token};

const TOKEN: Token = Token(0);

/// A socket whose readiness changes and waker signals can be awaited.
pub(crate) struct Readiness {
    poll: Poll,
    events: Events,
    watched: Option<mio::net::TcpStream>,
}

/// Interrupts a [`Readiness`] wait from another thread. Dropping it wakes the
/// wait one last time so the waiter can observe that its producer is gone.
pub(crate) struct Waker(mio::Waker);

impl Waker {
    pub(crate) fn wake(&self) -> io::Result<()> {
        self.0.wake()
    }
}

impl Drop for Waker {
    fn drop(&mut self) {
        let _ = self.0.wake();
    }
}

impl Readiness {
    pub(crate) fn new() -> io::Result<(Self, Waker)> {
        let poll = Poll::new()?;
        let waker = mio::Waker::new(poll.registry(), TOKEN)?;
        let readiness = Self {
            poll,
            events: Events::with_capacity(8),
            watched: None,
        };
        Ok((readiness, Waker(waker)))
    }

    /// Switches `socket` to non-blocking mode and watches it for readability.
    pub(crate) fn watch(&mut self, socket: &TcpStream) -> io::Result<()> {
        socket.set_nonblocking(true)?;
        let mut watched = mio::net::TcpStream::from_std(socket.try_clone()?);
        self.poll
            .registry()
            .register(&mut watched, TOKEN, Interest::READABLE)?;
        self.watched = Some(watched);
        Ok(())
    }

    /// Blocks until the socket is readable or the waker fires, failing with
    /// `TimedOut` once `deadline` passes.
    pub(crate) fn wait(&mut self, deadline: Option<Instant>) -> io::Result<()> {
        loop {
            let timeout =
                deadline.map(|deadline| deadline.saturating_duration_since(Instant::now()));
            match self.poll.poll(&mut self.events, timeout) {
                Ok(()) if timeout.is_some() && self.events.is_empty() => {
                    return Err(io::Error::from(io::ErrorKind::TimedOut));
                }
                Ok(()) => return Ok(()),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error),
            }
        }
    }

    /// Blocks like [`Self::wait`], also returning once the socket becomes writable.
    pub(crate) fn wait_writable(&mut self, deadline: Instant) -> io::Result<()> {
        self.set_interest(Interest::READABLE | Interest::WRITABLE)?;
        let waited = self.wait(Some(deadline));
        self.set_interest(Interest::READABLE)?;
        waited
    }

    fn set_interest(&mut self, interest: Interest) -> io::Result<()> {
        let watched = self.watched.as_mut().ok_or(io::ErrorKind::NotConnected)?;
        self.poll.registry().reregister(watched, TOKEN, interest)
    }
}

#[cfg(test)]
mod tests {
    use std::io::{ErrorKind, Write};
    use std::net::{TcpListener, TcpStream};
    use std::time::{Duration, Instant};

    use super::{Readiness, Waker};

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    fn watched_pair() -> Result<(Readiness, Waker, TcpStream), Box<dyn std::error::Error>> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let client = TcpStream::connect(listener.local_addr()?)?;
        let (server, _) = listener.accept()?;
        let (mut readiness, waker) = Readiness::new()?;
        readiness.watch(&server)?;
        Ok((readiness, waker, client))
    }

    fn soon() -> Instant {
        Instant::now() + Duration::from_millis(20)
    }

    #[test]
    fn times_out_when_nothing_happens() -> TestResult {
        let (mut readiness, _waker, _client) = watched_pair()?;
        let error = readiness.wait(Some(soon()));
        assert!(error.is_err_and(|error| error.kind() == ErrorKind::TimedOut));
        Ok(())
    }

    #[test]
    fn returns_when_woken_or_when_the_waker_is_dropped() -> TestResult {
        let (mut readiness, waker, _client) = watched_pair()?;
        waker.wake()?;
        readiness.wait(None)?;
        drop(waker);
        readiness.wait(None)?;
        Ok(())
    }

    #[test]
    fn returns_when_data_arrives() -> TestResult {
        let (mut readiness, _waker, mut client) = watched_pair()?;
        client.write_all(b"x")?;
        readiness.wait(None)?;
        Ok(())
    }

    #[test]
    fn stops_watching_writability_after_a_writable_wait() -> TestResult {
        let (mut readiness, _waker, _client) = watched_pair()?;
        readiness.wait_writable(soon())?;
        let error = readiness.wait(Some(soon()));
        assert!(error.is_err_and(|error| error.kind() == ErrorKind::TimedOut));
        Ok(())
    }
}
