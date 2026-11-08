//! A queue of files the OS asked the app to open. The macOS Apple Event callback has no access
//! to the `gpui` context and can fire before the window exists (cold launch by double click),
//! so it only pushes here, and a task spawned once the window is created awaits [`OpenQueue::next`].
//! The task sleeps until a push wakes it: nothing polls while the app is idle.

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::{Context, Poll, Waker};

#[derive(Default)]
struct Inner {
    paths: Vec<PathBuf>,
    waker: Option<Waker>,
}

/// Shared handle to the queue, cheap to clone.
#[derive(Clone, Default)]
pub struct OpenQueue(Arc<Mutex<Inner>>);

impl OpenQueue {
    /// Adds `paths` and wakes the waiting task, if any. Empty input does nothing.
    pub fn push(&self, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }
        let waker = {
            let mut inner = self.lock();
            inner.paths.extend(paths);
            inner.waker.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
    }

    /// Resolves with everything queued so far, waiting while the queue is empty.
    pub fn next(&self) -> Next {
        Next(self.clone())
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        // The state is a plain list, still valid after a panic elsewhere.
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// The future returned by [`OpenQueue::next`].
pub struct Next(OpenQueue);

impl Future for Next {
    type Output = Vec<PathBuf>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut inner = self.0.lock();
        if inner.paths.is_empty() {
            inner.waker = Some(cx.waker().clone());
            Poll::Pending
        } else {
            Poll::Ready(std::mem::take(&mut inner.paths))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::task::Wake;

    use super::*;

    struct Counter(AtomicUsize);

    impl Wake for Counter {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn items_pushed_before_the_first_wait_are_delivered() {
        let queue = OpenQueue::default();
        queue.push(vec![PathBuf::from("/a")]);
        queue.push(vec![PathBuf::from("/b")]);
        let counter = Arc::new(Counter(AtomicUsize::new(0)));
        let waker = Waker::from(counter);
        let mut cx = Context::from_waker(&waker);
        let mut next = Box::pin(queue.next());
        assert_eq!(
            next.as_mut().poll(&mut cx),
            Poll::Ready(vec![PathBuf::from("/a"), PathBuf::from("/b")])
        );
    }

    #[test]
    fn a_push_wakes_a_pending_wait() {
        let queue = OpenQueue::default();
        let counter = Arc::new(Counter(AtomicUsize::new(0)));
        let waker = Waker::from(counter.clone());
        let mut cx = Context::from_waker(&waker);
        let mut next = Box::pin(queue.next());
        assert_eq!(next.as_mut().poll(&mut cx), Poll::Pending);
        queue.push(Vec::new());
        assert_eq!(counter.0.load(Ordering::SeqCst), 0);
        queue.push(vec![PathBuf::from("/a")]);
        assert_eq!(counter.0.load(Ordering::SeqCst), 1);
        assert_eq!(
            next.as_mut().poll(&mut cx),
            Poll::Ready(vec![PathBuf::from("/a")])
        );
    }
}
