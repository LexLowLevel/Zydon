// Oneshot channel.
//
// Single-value async channel: send() resolves the receiver's future exactly once.
// Used for syscall replies and one-shot notifications.

use core::cell::UnsafeCell;
use core::future::Future;
use core::pin::Pin;
use core::sync::atomic::{AtomicU32, AtomicU8, Ordering};
use core::task::{Context, Poll, Waker};

/// Channel state machine.
const EMPTY: u8 = 0;        // no value, no receiver waiting
const WAITING: u8 = 1;      // receiver parked
const READY: u8 = 2;        // value sent, ready for delivery
const CONSUMED: u8 = 3;     // value delivered
const CLOSED: u8 = 4;       // sender dropped without sending

/// Create a new oneshot channel. Returns (Sender, Receiver).
pub fn oneshot<T>() -> (Sender<T>, Receiver<T>) {
    let shared_box = Box::new(SharedBox {
        refcount: AtomicU32::new(1),
        inner: OneshotInner {
            state: AtomicU8::new(EMPTY),
            value: UnsafeCell::new(None),
            recv_waker: UnsafeCell::new(None),
        },
    });
    let ptr = Box::into_raw(shared_box);
    let shared = Shared { ptr };
    (Sender { shared: Some(shared.clone()) }, Receiver { shared: Some(shared) })
}

/// Internal shared state, reference counted manually.
struct OneshotInner {
    state: AtomicU8,
    value: UnsafeCell<Option<T>>,
    recv_waker: UnsafeCell<Option<Waker>>,
}

// SAFETY: UnsafeCell fields are accessed under the atomic state protocol.
// Only one side writes at a time.
unsafe impl Send for OneshotInner {}
unsafe impl Sync for OneshotInner {}

/// Heap-allocated refcounted wrapper around the inner state.
struct SharedBox {
    refcount: AtomicU32,
    inner: OneshotInner,
}

struct Shared {
    ptr: *mut SharedBox,
}

impl Shared {
    fn inner(&self) -> &OneshotInner {
        // SAFETY: ptr is valid for the lifetime of all ref-counted holders.
        unsafe { &(*self.ptr).inner }
    }
}

impl Clone for Shared {
    fn clone(&self) -> Self {
        // SAFETY: ptr is valid; refcount ensures it stays alive.
        unsafe { (*self.ptr).refcount.fetch_add(1, Ordering::Relaxed); }
        Shared { ptr: self.ptr }
    }
}

impl Drop for Shared {
    fn drop(&mut self) {
        // SAFETY: ptr is valid. Last reference frees the allocation.
        unsafe {
            if (*self.ptr).refcount.fetch_sub(1, Ordering::Release) == 1 {
                drop(Box::from_raw(self.ptr));
            }
        }
    }
}

/// The sending half of a oneshot channel.
pub struct Sender<T> {
    shared: Option<Shared>,
}

/// The receiving half of a oneshot channel.
pub struct Receiver<T> {
    shared: Option<Shared>,
}

/// Error returned when sending fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendError<T> {
    /// The receiver was dropped before the value was sent.
    Closed(T),
}

/// Error returned when receiving fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecvError {
    /// The sender was dropped without sending a value.
    Closed,
}

impl<T> Sender<T> {
    /// Send a value. Returns Err if receiver was dropped.
    pub fn send(mut self, value: T) -> Result<(), SendError<T>> {
        let shared = self.shared.take().unwrap();
        let inner = shared.inner();

        // Store the value.
        // SAFETY: we are the only writer, state machine ensures receiver hasn't read.
        unsafe {
            *inner.value.get() = Some(value);
        }

        loop {
            let state = inner.state.load(Ordering::Acquire);
            match state {
                EMPTY => {
                    if inner.state.compare_exchange(
                        EMPTY,
                        READY,
                        Ordering::Release,
                        Ordering::Relaxed,
                    ).is_ok() {
                        return Ok(());
                    }
                    // CAS failed, retry.
                }
                WAITING => {
                    // Receiver is parked, transition to READY and wake it.
                    inner.state.store(READY, Ordering::Release);
                    // SAFETY: receiver is parked and owns the waker field.
                    unsafe {
                        if let Some(waker) = (*inner.recv_waker.get()).take() {
                            waker.wake();
                        }
                    }
                    return Ok(());
                }
                _ => {
                    unreachable!("oneshot send: invalid state {}", state);
                }
            }
        }
    }
}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        if let Some(shared) = self.shared.take() {
            let inner = shared.inner();
            let state = inner.state.load(Ordering::Acquire);
            if state == WAITING {
                // Signal closure and wake the receiver.
                inner.state.store(CLOSED, Ordering::Release);
                // SAFETY: receiver is parked, owns the waker.
                unsafe {
                    if let Some(waker) = (*inner.recv_waker.get()).take() {
                        waker.wake();
                    }
                }
            } else if state == EMPTY {
                inner.state.store(CLOSED, Ordering::Release);
            }
        }
    }
}

impl<T> Receiver<T> {
    /// Receive the value. Returns a future that resolves on send.
    pub fn recv(mut self) -> RecvFuture<T> {
        let shared = self.shared.take().unwrap();
        RecvFuture { shared, registered: false }
    }
}

/// Future returned by `Receiver::recv()`.
pub struct RecvFuture<T> {
    shared: Shared,
    registered: bool,
}

impl<T> Future for RecvFuture<T> {
    type Output = Result<T, RecvError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let inner = self.shared.inner();
        let state = inner.state.load(Ordering::Acquire);

        match state {
            READY => {
                // Value available, take it.
                inner.state.store(CONSUMED, Ordering::Release);
                // SAFETY: we are the sole reader after READY.
                let value = unsafe { (*inner.value.get()).take() };
                match value {
                    Some(v) => Poll::Ready(Ok(v)),
                    None => unreachable!("oneshot recv: READY but no value"),
                }
            }
            CLOSED => Poll::Ready(Err(RecvError::Closed)),
            EMPTY | WAITING => {
                if !self.registered {
                    // SAFETY: we store the waker, receiver owns this field.
                    unsafe {
                        *inner.recv_waker.get() = Some(cx.waker().clone());
                    }
                    // Try to transition to WAITING.
                    let _ = inner.state.compare_exchange(
                        EMPTY,
                        WAITING,
                        Ordering::Release,
                        Ordering::Relaxed,
                    );
                    self.registered = true;
                } else {
                    // Update waker in case executor moved us.
                    // SAFETY: same as above.
                    unsafe {
                        *inner.recv_waker.get() = Some(cx.waker().clone());
                    }
                }
                // Re-check after registering.
                let state = inner.state.load(Ordering::Acquire);
                if state == READY {
                    inner.state.store(CONSUMED, Ordering::Release);
                    let value = unsafe { (*inner.value.get()).take() };
                    match value {
                        Some(v) => Poll::Ready(Ok(v)),
                        None => unreachable!("oneshot recv: READY but no value"),
                    }
                } else if state == CLOSED {
                    Poll::Ready(Err(RecvError::Closed))
                } else {
                    Poll::Pending
                }
            }
            _ => unreachable!("oneshot recv: invalid state {}", state),
        }
    }
}