//! Conservative polling wrapper.
//!
//! Sometimes, you have a future that itself contains smaller futures. When the larger future is
//! polled, it polls those child futures to see if any of them have made progress. This can be
//! inefficient if polling such a future is expensive; when the big future is woken up, it is
//! usually because _one_ of its child futures was notified, and ideally only that one future
//! should be polled. Polling the other child futures that were _not_ notified is wasting precious
//! cycles.
//!
//! This crate provides a wrapper for `Future` types, and other types that you may wish to call
//! `poll`-like methods on. When you poll the inner `Future` through [`Strawpoll`] (or using
//! [`Strawpoll::poll_fn`]), that poll call will immediately return with `Poll::Pending` if the
//! contained future was not actually notified. In that case it will _not_ poll the inner future.
//!
//! Consider the following example where `TrackPolls` is some wrapper type that lets you measure
//! how many times `poll` was called on it, and `spawn` is a method that lets you poll a `Future`
//! without constructing a `Context` yourself.
//!
//! ```
//! # use std::{
//! #     future::Future,
//! #     pin::Pin,
//! #     task::{Context, Poll, Waker},
//! # };
//! # use tokio_test::{assert_pending, assert_ready, task::spawn};
//! # use tokio::sync::oneshot;
//! #
//! # struct TrackPolls<F> {
//! #     npolls: usize,
//! #     f: F,
//! # }
//! #
//! # impl<F> TrackPolls<F> {
//! #     fn new(f: F) -> Self {
//! #         Self { npolls: 0, f }
//! #     }
//! # }
//! #
//! # impl<F> Future for TrackPolls<F>
//! # where
//! #     F: Future,
//! # {
//! #     type Output = F::Output;
//! #     fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
//! #         // safety: we do not move f
//! #         let this = unsafe { self.get_unchecked_mut() };
//! #         this.npolls += 1;
//! #         // safety: we are pinned, and so is f
//! #         unsafe { Pin::new_unchecked(&mut this.f) }.poll(cx)
//! #     }
//! # }
//! #
//! #
//! use strawpoll::Strawpoll;
//!
//! let (tx, rx) = oneshot::channel();
//! let mut rx = spawn(Strawpoll::new(TrackPolls::new(rx)));
//! assert_pending!(rx.poll());
//! assert_pending!(rx.poll());
//! assert_pending!(rx.poll());
//! // one poll must go through to register the underlying future
//! // but the _other_ calls to poll should do nothing, since no notify has happened
//! assert_eq!(rx.npolls, 1);
//! tx.send(()).unwrap();
//! assert_ready!(rx.poll()).unwrap();
//! // now there _was_ a notify, so the inner poll _should_ be called
//! assert_eq!(rx.npolls, 2);
//! ```
#![warn(rust_2018_idioms)]
#![deny(
    missing_docs,
    missing_debug_implementations,
    unreachable_pub,
    intra_doc_link_resolution_failure
)]

use std::sync::{
    atomic::{AtomicBool, Ordering::SeqCst},
    Arc,
};
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll, Waker},
};

/// Polling wrapper that avoids spurious calls to `poll` on `F`.
#[derive(Debug)]
pub struct Strawpoll<F> {
    future: F,
    waker: Option<Arc<TrackWake>>,
}

impl<F> From<F> for Strawpoll<F> {
    fn from(f: F) -> Self {
        Self::new(f)
    }
}

impl<F> Strawpoll<F> {
    /// Wrap `f` to avoid spurious polling on it.
    pub fn new(f: F) -> Self {
        Self {
            future: f,
            waker: None,
        }
    }

    /// Call `poll_fn` with `F` pinned only if `F` really needs to be polled.
    ///
    /// Specifically, `poll_fn` will only be called if:
    ///
    ///  - `F` has never been polled; or
    ///  - `cx` contains a new waker; or
    ///  - `F` was woken up.
    pub fn poll_fn<P, R>(self: Pin<&mut Self>, cx: &mut Context<'_>, poll_fn: P) -> Poll<R>
    where
        P: FnOnce(Pin<&mut F>, &mut Context<'_>) -> Poll<R>,
    {
        // safety: we will not move F
        let this = unsafe { self.get_unchecked_mut() };

        let cx_waker = cx.waker();
        if this.waker.is_none() || !cx_waker.will_wake(&this.waker.as_ref().unwrap().real) {
            this.waker = Some(Arc::new(TrackWake {
                real: cx_waker.clone(),
                awoken: AtomicBool::new(true),
            }));
        }

        let waker = this.waker.as_ref().unwrap();
        if !waker.awoken.swap(false, SeqCst) {
            return Poll::Pending;
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
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.poll_fn(cx, |f, cx| f.poll(cx))
    }
}

#[derive(Debug)]
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
        npolls: usize,
        f: F,
    }

    impl<F> TrackPolls<F> {
        fn new(f: F) -> Self {
            Self { npolls: 0, f }
        }
    }

    impl<F> Future for TrackPolls<F>
    where
        F: Future,
    {
        type Output = F::Output;
        fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            // safety: we do not move f
            let this = unsafe { self.get_unchecked_mut() };
            this.npolls += 1;
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
        let mut rx = spawn(Strawpoll::from(TrackPolls::new(rx)));
        assert_pending!(rx.poll());
        assert_pending!(rx.poll());
        assert_pending!(rx.poll());
        // one poll must go through to register the underlying future
        // but the _other_ calls to poll should do nothing, since no notify has happened
        assert_eq!(rx.npolls, 1);
        rx.npolls = 0;
        tx.send(()).unwrap();
        assert_ready!(rx.poll()).unwrap();
        // now there _was_ a notify, so the inner poll _should_ be called
        assert_eq!(rx.npolls, 1);
    }

    #[test]
    fn it_handles_changing_wakers() {
        let (tx, rx) = oneshot::channel();
        let mut rx = spawn(Strawpoll::from(TrackPolls::new(rx)));
        assert_pending!(rx.poll());
        assert_pending!(rx.poll());
        assert_eq!(rx.npolls, 1);
        // change wakers
        let mut rx = spawn(rx.into_inner());
        assert_pending!(rx.poll());
        assert_pending!(rx.poll());
        // after the waker changs, we _must_ poll again to register with the new waker
        assert_eq!(rx.npolls, 2);
        // change wakers again and wake
        let mut rx = spawn(rx.into_inner());
        tx.send(()).unwrap();
        assert_ready!(rx.poll()).unwrap();
        // now there _was_ a notify, so the inner poll _should_ be called
        assert_eq!(rx.npolls, 3);
    }
}
