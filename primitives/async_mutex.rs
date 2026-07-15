// Async mutex with priority inheritance.
//
// Never blocks synchronously. lock() returns a future that parks
// the task until the holder releases. PI prevents unbounded
// priority inversion.

use core::cell::UnsafeCell;
use core::future::Future;
use core::ops::{Deref, DerefMut};
use core::pin::Pin;
use core::sync::atomic::{AtomicI32, AtomicU32, Ordering};
use core::task::{Context, Poll};

use crate::primitives::wait_queue::{WaitQueue, WaitNode};

const UNLOCKED: u32 = 0;
const LOCKED_NO_WAITERS: u32 = 1;
const LOCKED_HAS_WAITERS: u32 = 2;

/// Async mutex with priority inheritance. Guard releases on drop.
pub struct AsyncMutex<T> {
    state: AtomicU32,
    waiters: WaitQueue,
    holder_priority: AtomicI32,     // for PI boosting
    holder_base_priority: AtomicI32,
    data: UnsafeCell<T>,
}

// SAFETY: provides its own mutual exclusion via async locking.
unsafe impl<T: Send> Send for AsyncMutex<T> {}
unsafe impl<T: Send> Sync for AsyncMutex<T> {}

pub struct MutexGuard<'a, T> {
    mutex: &'a AsyncMutex<T>,
}

impl<'a, T> Deref for MutexGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { &*self.mutex.data.get() }
    }
}

impl<'a, T> DerefMut for MutexGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.mutex.data.get() }
    }
}

impl<'a, T> Drop for MutexGuard<'a, T> {
    fn drop(&mut self) {
        self.mutex.unlock_inner();
    }
}

impl<T> AsyncMutex<T> {
    /// Create a new async mutex protecting `data`.
    pub const fn new(data: T) -> Self {
        Self {
            state: AtomicU32::new(UNLOCKED),
            waiters: WaitQueue::new(),
            holder_priority: AtomicI32::new(0),
            holder_base_priority: AtomicI32::new(0),
            data: UnsafeCell::new(data),
        }
    }

    /// Acquire the lock, returning a future that completes when the lock
    /// is held.
    pub fn lock(&self) -> MutexLockFuture<'_, T> {
        MutexLockFuture {
            mutex: self,
            wait_node: WaitNode::new(),
            registered: false,
        }
    }

    /// Try to acquire the lock without waiting.
    ///
    /// Returns `Some(guard)` if the lock was free, `None` otherwise.
    pub fn try_lock(&self) -> Option<MutexGuard<'_, T>> {
        let prev = self.state.compare_exchange(
            UNLOCKED,
            LOCKED_NO_WAITERS,
            Ordering::Acquire,
            Ordering::Relaxed,
        );
        match prev {
            Ok(_) => Some(MutexGuard { mutex: self }),
            Err(_) => None,
        }
    }

    fn unlock_inner(&self) {
        // Undo PI boost.
        let _base = self.holder_base_priority.load(Ordering::Relaxed);
        self.holder_priority.store(0, Ordering::Relaxed);

        if self.waiters.is_empty() {
            // Fast path: no waiters.
            let prev = self.state.swap(UNLOCKED, Ordering::Release);
            debug_assert_ne!(prev, UNLOCKED, "unlock of unlocked mutex");
        } else {
            // Has waiters. Transition to UNLOCKED and wake one.
            self.state.store(UNLOCKED, Ordering::Release);
            self.waiters.wake_one();
        }
    }

    fn apply_priority_inheritance(&self, waiter_priority: i32) {
        // Boost holder's effective priority to at least waiter_priority.
        loop {
            let current = self.holder_priority.load(Ordering::Relaxed);
            if current >= waiter_priority {
                break;
            }
            match self.holder_priority.compare_exchange_weak(
                current,
                waiter_priority,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(_) => continue,
            }
        }
        // TODO: also update the scheduler's run queue position for the holder.
    }
}

/// Future returned by AsyncMutex::lock().
pub struct MutexLockFuture<'a, T> {
    mutex: &'a AsyncMutex<T>,
    wait_node: WaitNode,
    registered: bool,
}

impl<'a, T> Future for MutexLockFuture<'a, T> {
    type Output = MutexGuard<'a, T>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // Fast path: try to grab it immediately.
        let prev = self.mutex.state.compare_exchange(
            UNLOCKED,
            LOCKED_NO_WAITERS,
            Ordering::Acquire,
            Ordering::Relaxed,
        );
        if prev.is_ok() {
            return Poll::Ready(MutexGuard { mutex: self.mutex });
        }

        // Slow path: register on the wait queue and park.
        if !self.registered {
            self.wait_node.set_waker(cx.waker().clone());
            self.wait_node.set_priority(0); // TODO: get from TCB

            // Mark that there are waiters.
            self.mutex
                .state
                .store(LOCKED_HAS_WAITERS, Ordering::Relaxed);

            // SAFETY: wait_node is pinned inside this future.
            unsafe {
                self.mutex.waiters.enqueue(&mut self.wait_node);
            }
            self.registered = true;

            // Apply PI: boost the holder's priority.
            self.mutex.apply_priority_inheritance(self.wait_node.priority());
        }

        // Try once more after enqueuing.
        let prev = self.mutex.state.compare_exchange(
            UNLOCKED,
            LOCKED_HAS_WAITERS,
            Ordering::Acquire,
            Ordering::Relaxed,
        );
        if prev.is_ok() {
            // Got it. Remove from wait queue.
            if self.registered {
                // SAFETY: we just acquired the lock, node is still valid.
                unsafe {
                    self.mutex.waiters.remove(&mut self.wait_node);
                }
                self.registered = false;
            }
            return Poll::Ready(MutexGuard { mutex: self.mutex });
        }

        Poll::Pending
    }
}

impl<'a, T> Drop for MutexLockFuture<'a, T> {
    fn drop(&mut self) {
        if self.registered {
            // SAFETY: node is still valid during drop (future being cancelled).
            unsafe {
                self.mutex.waiters.remove(&mut self.wait_node);
            }
        }
    }
}
