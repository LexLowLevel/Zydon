// Ticket spinlock with exponential backoff.
// FIFO order, <1us hold time. For interrupt handlers and executor internals.

use core::sync::atomic::{AtomicU32, Ordering};
use core::cell::UnsafeCell;
use core::hint;

/// Ticket spinlock. NOT Send/Sync across interrupt boundaries.
/// Critical section must be bounded (no await points).
pub struct Spinlock<T> {
    next_ticket: AtomicU32,  // next ticket to hand out
    now_serving: AtomicU32,  // ticket currently being served
    data: UnsafeCell<T>,
}

// SAFETY: Spinlock provides mutual exclusion.
unsafe impl<T: Send> Send for Spinlock<T> {}
unsafe impl<T: Send> Sync for Spinlock<T> {}

/// RAII spinlock guard.
pub struct SpinlockGuard<'a, T> {
    lock: &'a Spinlock<T>,
}

impl<'a, T> core::ops::Deref for SpinlockGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { &*self.lock.data.get() }
    }
}

impl<'a, T> core::ops::DerefMut for SpinlockGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<'a, T> Drop for SpinlockGuard<'a, T> {
    fn drop(&mut self) {
        // Advance serving ticket to release the lock.
        self.lock.now_serving.fetch_add(1, Ordering::Release);
    }
}

impl<T> Spinlock<T> {
    pub const fn new(data: T) -> Self {
        Self {
            next_ticket: AtomicU32::new(0),
            now_serving: AtomicU32::new(0),
            data: UnsafeCell::new(data),
        }
    }

    /// Acquire with exponential backoff.
    pub fn lock(&self) -> SpinlockGuard<'_, T> {
        let ticket = self.next_ticket.fetch_add(1, Ordering::Relaxed);
        self.wait_for_ticket(ticket);
        SpinlockGuard { lock: self }
    }

    /// Try to acquire without spinning.
    pub fn try_lock(&self) -> Option<SpinlockGuard<'_, T>> {
        let serving = self.now_serving.load(Ordering::Acquire);
        let next = self.next_ticket.load(Ordering::Relaxed);
        if next != serving {
            return None;
        }
        match self.next_ticket.compare_exchange(
            next,
            next + 1,
            Ordering::Acquire,
            Ordering::Relaxed,
        ) {
            Ok(_) => Some(SpinlockGuard { lock: self }),
            Err(_) => None,
        }
    }

    #[inline]
    pub fn is_locked(&self) -> bool {
        let serving = self.now_serving.load(Ordering::Relaxed);
        let next = self.next_ticket.load(Ordering::Relaxed);
        next != serving
    }

    fn wait_for_ticket(&self, ticket: u32) {
        let mut backoff = 1u32;
        loop {
            let serving = self.now_serving.load(Ordering::Acquire);
            if serving == ticket {
                return;
            }
            // Backoff with pause.
            for _ in 0..backoff {
                hint::spin_loop();
            }
            backoff = (backoff * 2).min(256);
        }
    }
}
