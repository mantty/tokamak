use std::cell::Cell;
use std::future::{Future, poll_fn};
use std::pin::pin;
use std::rc::Rc;
use std::task::Poll;

use rquickjs::{AsyncRuntime, Ctx};

use crate::quickjs::Error;

#[derive(Clone, Default, rquickjs::JsLifetime)]
pub(crate) struct AwaitedPromise(Rc<Cell<Option<&'static str>>>);

pub(crate) struct PromiseWait {
    state: AwaitedPromise,
    previous: Option<&'static str>,
}

impl PromiseWait {
    pub(crate) fn enter(ctx: &Ctx<'_>, stage: &'static str) -> Option<Self> {
        let current = ctx.userdata::<AwaitedPromise>()?.clone();
        let previous = current.0.replace(Some(stage));
        Some(Self {
            state: current,
            previous,
        })
    }
}

impl Drop for PromiseWait {
    fn drop(&mut self) {
        self.state.0.set(self.previous);
    }
}

pub(crate) async fn run(
    runtime: &AsyncRuntime,
    awaited: &AwaitedPromise,
    request: impl Future<Output = Result<(), Error>>,
) -> Result<(), Error> {
    let mut request = pin!(request);
    poll_fn(|cx| {
        if let Poll::Ready(result) = request.as_mut().poll(cx) {
            return Poll::Ready(result);
        }
        let Some(stage) = awaited.0.get() else {
            return Poll::Pending;
        };
        // async_with has driven runnable JS jobs and native futures before yielding.
        // A pending promise with neither cannot ever settle. External transport waits
        // are not promise waits and remain live until the peer or lifecycle wakes them.
        let mut pending = pin!(runtime.is_job_pending());
        if matches!(pending.as_mut().poll(cx), Poll::Ready(false)) {
            return Poll::Ready(Err(Error::Engine(format!(
                "{stage}: promise did not settle"
            ))));
        }
        Poll::Pending
    })
    .await
}
