use std::sync::{
    atomic::{AtomicBool, Ordering::SeqCst},
    Arc,
};
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll, Waker},
};

pub struct Strawpoll<F> {
    future: F,
    waker: Option<Arc<TrackWake>>,
}

impl<F> Strawpoll<F> {
    pub fn new(f: F) -> Self {
        Self {
            future: f,
            waker: None,
        }
    }

    pub fn poll_fn<P, R>(self: Pin<&mut Self>, cx: &mut Context, poll_fn: P) -> Poll<R>
    where
        P: FnOnce(Pin<&mut F>, &mut Context) -> Poll<R>,
    {
        // safety: we will not move F
        let this = unsafe { self.get_unchecked_mut() };

        if this.waker.is_none() {
            this.waker = Some(Arc::new(TrackWake {
                real: cx.waker().clone(),
                awoken: AtomicBool::new(true),
            }))
        }
        let waker = this.waker.as_ref().unwrap();
        if !waker.awoken.swap(false, SeqCst) {
            return Poll::Pending;
        }
        if !cx.waker().will_wake(&waker.real) {
            todo!();
        }

        // safety: we are already pinned, and caller has no way to move us (or F) once we've
        // reached this point unless F: Unpin.
        let fpin = unsafe { Pin::new_unchecked(&mut this.future) };
        let wref = futures_task::waker_ref(waker);
        let mut cx = Context::from_waker(&*wref);
        poll_fn(fpin, &mut cx)
    }
}

impl<F> std::ops::Deref for Strawpoll<F> {
    type Target = F;
    fn deref(&self) -> &Self::Target {
        &self.future
    }
}

impl<F> std::ops::DerefMut for Strawpoll<F> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.future
    }
}

impl<F> Unpin for Strawpoll<F> where F: Unpin {}

impl<F> Future for Strawpoll<F>
where
    F: Future,
{
    type Output = F::Output;
    fn poll(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        self.poll_fn(cx, |f, cx| f.poll(cx))
    }
}

struct TrackWake {
    real: Waker,
    awoken: AtomicBool,
}

impl futures_task::ArcWake for TrackWake {
    fn wake_by_ref(arc_self: &Arc<Self>) {
        arc_self.awoken.store(true, SeqCst);
        arc_self.real.wake_by_ref();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::oneshot;
    use tokio_test::{assert_pending, assert_ready, task::spawn};

    struct TrackPolls<F> {
        n: usize,
        f: F,
    }

    impl<F> TrackPolls<F> {
        fn new(f: F) -> Self {
            Self { n: 0, f }
        }
    }

    impl<F> Future for TrackPolls<F>
    where
        F: Future,
    {
        type Output = F::Output;
        fn poll(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
            // safety: we do not move f
            let this = unsafe { self.get_unchecked_mut() };
            this.n += 1;
            // safety: we are pinned, and so is f
            unsafe { Pin::new_unchecked(&mut this.f) }.poll(cx)
        }
    }

    #[test]
    fn it_resolves() {
        let (tx, rx) = oneshot::channel();
        let mut rx = spawn(TrackPolls::new(rx));
        assert_pending!(rx.poll());
        tx.send(()).unwrap();
        assert_ready!(rx.poll()).unwrap();
    }

    #[test]
    fn it_only_polls_when_needed() {
        let (tx, rx) = oneshot::channel();
        let mut rx = spawn(Strawpoll::new(TrackPolls::new(rx)));
        assert_pending!(rx.poll());
        assert_pending!(rx.poll());
        assert_pending!(rx.poll());
        // one poll must go through to register the underlying future
        // but the _other_ calls to poll should do nothing, since no notify has happened
        assert_eq!(rx.n, 1);
        tx.send(()).unwrap();
        assert_ready!(rx.poll()).unwrap();
        // now there _was_ a notify, so the inner poll _should_ be called
        assert_eq!(rx.n, 2);
    }
}
