// Broadcast channel.
//
// Single-producer, multi-consumer. Each send() wakes all parked receivers
// with the same value. Uses a generation counter so receivers can tell
// when a new value has arrived.

use core::cell::UnsafeCell;
use core::future::Future;
use core::pin::Pin;
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use core::task::{Context, Poll, Waker};

use crate::primitives::wait_queue::{WaitQueue, WaitNode};

/// Create a new broadcast channel. Returns (Sender, Receiver).
pub fn broadcast<T: Clone>() -> (Sender<T>, Receiver<T>) {
    let shared = Box::new(BroadcastShared {
        refcount: AtomicU32::new(1),
        inner: BroadcastInner {
            generation: AtomicU64::new(0),
            value: UnsafeCell::new(None),
            waiters: WaitQueue::new(),
        },
    });
    let ptr = Box::into_raw(shared);

    let sender = Sender { ptr };
    let receiver = Receiver {
        ptr,
        last_seen: 0,
        wait_node: WaitNode::new(),
        registered: false,
    };
    (sender, receiver)
}

struct BroadcastShared {
    refcount: AtomicU32,
    inner: BroadcastInner,
}

struct BroadcastInner {
    generation: AtomicU64,
    value: UnsafeCell<Option<T>>,
    waiters: WaitQueue,
}

// SAFETY: generation counter protocol ensures only one side writes at a time.
unsafe impl<T: Send> Send for BroadcastInner {}
unsafe impl<T: Send> Sync for BroadcastInner {}

/// The sending half of a broadcast channel.
pub struct Sender<T> {
    ptr: *mut BroadcastShared,
}

impl<T> Sender<T> {
    fn inner(&self) -> &BroadcastInner {
        // SAFETY: ptr is valid for the lifetime of all ref-counted holders.
        unsafe { &(*self.ptr).inner }
    }
}

impl<T: Clone> Sender<T> {
    /// Broadcast a value to all receivers.
    pub fn send(&self, value: T) {
        let inner = self.inner();

        // SAFETY: we are the sole producer. No reader until generation bumps.
        unsafe {
            *inner.value.get() = Some(value);
        }

        // Advance generation, releasing the value store.
        inner.generation.fetch_add(1, Ordering::Release);

        inner.waiters.wake_all();
    }
}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        let inner = self.inner();
        // Signal closure.
        inner.generation.store(u64::MAX, Ordering::Release);
        inner.waiters.wake_all();
        // Release refcount.
        unsafe {
            if (*self.ptr).refcount.fetch_sub(1, Ordering::Release) == 1 {
                drop(Box::from_raw(self.ptr));
            }
        }
    }
}

/// The receiving half of a broadcast channel.
pub struct Receiver<T> {
    ptr: *mut BroadcastShared,
    last_seen: u64,
    wait_node: WaitNode,
    registered: bool,
}

impl<T> Receiver<T> {
    fn inner(&self) -> &BroadcastInner {
        // SAFETY: ptr is valid for the lifetime of all ref-counted holders.
        unsafe { &(*self.ptr).inner }
    }
}

impl<T: Clone> Receiver<T> {
    /// Subscribe a new receiver starting at the current generation.
    pub fn subscribe(&self) -> Receiver<T> {
        let inner = self.inner();
        // SAFETY: ptr is valid; refcount ensures it stays alive.
        unsafe { (*self.ptr).refcount.fetch_add(1, Ordering::Relaxed); }
        let gen = inner.generation.load(Ordering::Acquire);
        Receiver {
            ptr: self.ptr,
            last_seen: gen,
            wait_node: WaitNode::new(),
            registered: false,
        }
    }

    /// Receive the next broadcast value.
    pub fn recv(&mut self) -> BroadcastRecvFuture<'_, T> {
        BroadcastRecvFuture { receiver: self }
    }
}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        // Release refcount.
        unsafe {
            if (*self.ptr).refcount.fetch_sub(1, Ordering::Release) == 1 {
                drop(Box::from_raw(self.ptr));
            }
        }
    }
}

/// Future returned by `Receiver::recv()`.
pub struct BroadcastRecvFuture<'a, T> {
    receiver: &'a mut Receiver<T>,
}

impl<'a, T: Clone> Future for BroadcastRecvFuture<'a, T> {
    type Output = Result<T, BroadcastRecvError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let inner = self.receiver.inner();
        let current_gen = inner.generation.load(Ordering::Acquire);

        if current_gen == u64::MAX {
            return Poll::Ready(Err(BroadcastRecvError::Closed));
        }

        if current_gen > self.receiver.last_seen {
            // New value available.
            // SAFETY: generation advanced, producer is done writing.
            let value = unsafe {
                (*inner.value.get()).clone().expect("value must be set at non-zero generation")
            };
            self.receiver.last_seen = current_gen;
            return Poll::Ready(Ok(value));
        }

        // No new value, park and wait.
        if !self.receiver.registered {
            self.receiver.wait_node.set_waker(cx.waker().clone());
            // SAFETY: wait_node is embedded in the receiver, pinned by this future.
            unsafe {
                inner.waiters.enqueue(&mut self.receiver.wait_node);
            }
            self.receiver.registered = true;
        } else {
            // Update waker.
            self.receiver.wait_node.set_waker(cx.waker().clone());
        }

        // Re-check after registering.
        let current_gen = inner.generation.load(Ordering::Acquire);
        if current_gen == u64::MAX {
            // SAFETY: we registered, need to clean up.
            unsafe { inner.waiters.remove(&mut self.receiver.wait_node); }
            self.receiver.registered = false;
            return Poll::Ready(Err(BroadcastRecvError::Closed));
        }
        if current_gen > self.receiver.last_seen {
            // Value arrived between our check and registration.
            unsafe { inner.waiters.remove(&mut self.receiver.wait_node); }
            self.receiver.registered = false;
            let value = unsafe {
                (*inner.value.get()).clone().expect("value must be set")
            };
            self.receiver.last_seen = current_gen;
            return Poll::Ready(Ok(value));
        }

        Poll::Pending
    }
}

impl<'a, T> Drop for BroadcastRecvFuture<'a, T> {
    fn drop(&mut self) {
        if self.receiver.registered {
            let inner = self.receiver.inner();
            // SAFETY: wait_node is still valid during drop.
            unsafe {
                inner.waiters.remove(&mut self.receiver.wait_node);
            }
            self.receiver.registered = false;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BroadcastRecvError {
    Closed,
}