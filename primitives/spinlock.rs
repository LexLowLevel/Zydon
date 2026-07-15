// Ticket spinlock with exponential backoff.
//
// FIFO acquisition order prevents starvation. Only used in interrupt
// handlers and executor internals where yielding isn't possible.
// Hold time invariant: < 1us.

use core::sync::atomic::{AtomicU32, Ordering};
use core::cell::UnsafeCell;
use core::hint;

/// Ticket spinlock. NOT Send/Sync across interrupt boundaries.
/// Critical section must be bounded (no await points).
pub struct Spinlock<T> {
    state: AtomicU32, // low 16 = next ticket, high 16 = currently serving
    data: UnsafeCell<T>,
}

// SAFETY: Spinlock provides mutual exclusion.
unsafe impl<T: Send> Send for Spinlock<T> {}
unsafe impl<T: Send> Sync for Spinlock<T> {}

const TICKET_BITS: u32 = 16;
const TICKET_MASK: u32 = (1 << TICKET_BITS) - 1;

/// Guard that releases the spinlock on drop.
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
        let old = self.lock.state.load(Ordering::Relaxed);
        let new_serving = ((old >> TICKET_BITS) + 1) & TICKET_MASK;
        self.lock.state.store(
            (old & !(!0u32 << TICKET_BITS)) | (new_serving << TICKET_BITS),
            Ordering::Release,
        );
    }
}

impl<T> Spinlock<T> {
    pub const fn new(data: T) -> Self {
        Self {
            state: AtomicU32::new(0),
            data: UnsafeCell::new(data),
        }
    }

    /// Acquire the lock, spinning with exponential backoff.
    pub fn lock(&self) -> SpinlockGuard<'_, T> {
        let ticket = self.fetch_ticket();
        self.wait_for_ticket(ticket);
        SpinlockGuard { lock: self }
    }

    /// Try to acquire without spinning. Wait-free.
    pub fn try_lock(&self) -> Option<SpinlockGuard<'_, T>> {
        let state = self.state.load(Ordering::Relaxed);
        let next = state & TICKET_MASK;
        let serving = state >> TICKET_BITS;
        if next == serving {
            // Lock is free, try to claim it.
            let new_state = ((serving + 1) & TICKET_MASK) << TICKET_BITS
                | ((next + 1) & TICKET_MASK);
            match self.state.compare_exchange_weak(
                state,
                new_state,
                Ordering::Acquire,
                Ordering::Relaxed,
            ) {
                Ok(_) => Some(SpinlockGuard { lock: self }),
                Err(_) => None,
            }
        } else {
            None
        }
    }

    pub fn is_locked(&self) -> bool {
        let state = self.state.load(Ordering::Relaxed);
        let next = state & TICKET_MASK;
        let serving = state >> TICKET_BITS;
        next != serving
    }

    fn fetch_ticket(&self) -> u32 {
        let old = self.state.fetch_add(1, Ordering::Relaxed);
        old & TICKET_MASK
    }

    fn wait_for_ticket(&self, ticket: u32) {
        let mut backoff = 1u32;
        loop {
            let serving = self.state.load(Ordering::Acquire) >> TICKET_BITS;
            if serving == (ticket & TICKET_MASK) {
                return;
            }
            // Backoff with pause hint.
            for _ in 0..backoff {
                hint::spin_loop();
            }
            backoff = (backoff * 2).min(256);
        }
    }
}

/// Spinlock that works in a static context (new is already const).
pub type StaticSpinlock<T> = Spinlock<T>;