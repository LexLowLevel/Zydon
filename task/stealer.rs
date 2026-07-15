// Chase-Lev work-stealing deque.
//
// Each core owns a Worker (LIFO push/pop) and publishes a Stealer (FIFO).
// When a core runs out of work, it steals half of another core's deque.
//
// Reference: Chase and Lev, "Dynamic Circular Work-Stealing Deque", SPAA 2005.

use core::sync::atomic::{AtomicIsize, AtomicUsize, Ordering};
use core::cell::UnsafeCell;

/// Minimum deque capacity (must be power of 2).
const MIN_CAPACITY: usize = 64;

/// Steal half the deque, per Cilk convention.
const MAX_STEAL_BATCH: usize = 128;

/// A work-stealing deque. Worker is the owner, Stealer is the thief.
pub struct Worker<T: Copy> {
    inner: *mut DequeInner<T>,
}

pub struct Stealer<T: Copy> {
    inner: *mut DequeInner<T>,
}

struct DequeInner<T: Copy> {
    bottom: AtomicIsize, // next push slot (worker-only)
    top: AtomicIsize,    // oldest element (shared, CAS'd)
    buffer: UnsafeCell<DequeBuffer<T>>,
}

struct DequeBuffer<T: Copy> {
    data: *mut T,
    log_cap: u32, // log2(capacity), for fast modulo
    cap: usize,
}

// SAFETY: bottom is worker-only, top is CAS'd by all, buffer is written
// at exclusively-owned indices.
unsafe impl<T: Copy> Send for DequeInner<T> {}
unsafe impl<T: Copy> Sync for DequeInner<T> {}

impl<T: Copy> DequeBuffer<T> {
    fn new(log_cap: u32) -> Self {
        let cap = 1usize << log_cap;
        let layout = core::alloc::Layout::array::<T>(cap).expect("deque capacity overflow");
        let data = unsafe { alloc::alloc::alloc(layout) } as *mut T;
        assert!(!data.is_null(), "deque allocation failed");
        Self { data, log_cap, cap }
    }

    fn mask(&self) -> usize {
        self.cap - 1
    }

    /// Read element at index `i` (mod capacity).
    /// SAFETY: slot must have been written and not yet overwritten.
    unsafe fn get(&self, i: isize) -> T {
        *self.data.add(i as usize & self.mask())
    }

    /// Write element at index `i` (mod capacity).
    /// SAFETY: slot must be exclusively owned by the writer.
    unsafe fn put(&self, i: isize, val: T) {
        *self.data.add(i as usize & self.mask()) = val;
    }
}

impl<T: Copy> Drop for DequeBuffer<T> {
    fn drop(&mut self) {
        let layout = core::alloc::Layout::array::<T>(self.cap).expect("deque capacity");
        unsafe {
            alloc::alloc::dealloc(self.data as *mut u8, layout);
        }
    }
}

/// Create a new work-stealing deque. Returns (Worker, Stealer).
pub fn deque<T: Copy>() -> (Worker<T>, Stealer<T>) {
    let inner = Box::leak(Box::new(DequeInner {
        bottom: AtomicIsize::new(0),
        top: AtomicIsize::new(0),
        buffer: UnsafeCell::new(DequeBuffer::new(MIN_CAPACITY.trailing_zeros())),
    }));
    (Worker { inner }, Stealer { inner })
}

impl<T: Copy> Worker<T> {
    /// Push to the bottom (LIFO end).
    pub fn push(&self, val: T) {
        let inner = unsafe { &*self.inner };
        let b = inner.bottom.load(Ordering::Relaxed);
        let t = inner.top.load(Ordering::Acquire);
        let buf = unsafe { &*inner.buffer.get() };

        let size = b - t;
        if size >= buf.cap as isize {
            self.grow(buf, t, b);
            let buf = unsafe { &*inner.buffer.get() };
            unsafe { buf.put(b, val) };
        } else {
            unsafe { buf.put(b, val) };
        }

        // Ensure the buffer write is visible before updating bottom.
        core::sync::atomic::fence(Ordering::Release);
        inner.bottom.store(b + 1, Ordering::Relaxed);
    }

    /// Pop from the bottom (LIFO end). Returns None if empty.
    pub fn pop(&self) -> Option<T> {
        let inner = unsafe { &*self.inner };
        let b = inner.bottom.load(Ordering::Relaxed) - 1;
        inner.bottom.store(b, Ordering::Relaxed);

        core::sync::atomic::fence(Ordering::SeqCst);

        let t = inner.top.load(Ordering::Relaxed);
        if t <= b {
            let buf = unsafe { &*inner.buffer.get() };
            let val = unsafe { buf.get(b) };
            if t == b {
                // Last element, compete with stealers via CAS.
                match inner.top.compare_exchange(
                    t,
                    t + 1,
                    Ordering::SeqCst,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => {
                        inner.bottom.store(b + 1, Ordering::Relaxed);
                        Some(val)
                    }
                    Err(_) => {
                        // A stealer got it first.
                        inner.bottom.store(b + 1, Ordering::Relaxed);
                        None
                    }
                }
            } else {
                Some(val)
            }
        } else {
            // Deque was empty.
            inner.bottom.store(b + 1, Ordering::Relaxed);
            None
        }
    }

    /// Approximate length.
    pub fn len(&self) -> usize {
        let inner = unsafe { &*self.inner };
        let b = inner.bottom.load(Ordering::Relaxed);
        let t = inner.top.load(Ordering::Relaxed);
        (b - t).max(0) as usize
    }

    fn grow(&self, old_buf: &DequeBuffer<T>, t: isize, b: isize) {
        let inner = unsafe { &*self.inner };
        let new_log_cap = old_buf.log_cap + 1;
        let new_buf = DequeBuffer::new(new_log_cap);

        for i in t..b {
            unsafe {
                let val = old_buf.get(i);
                new_buf.put(i, val);
            }
        }

        // Replace the buffer. Only the owner (Worker) grows it.
        // SAFETY: No stealer reads during growth since we hold the only
        // mutable reference via UnsafeCell.
        unsafe {
            *inner.buffer.get() = new_buf;
        }
    }
}

impl<T: Copy> Stealer<T> {
    /// Steal one element from the top (FIFO end).
    /// Returns None if empty or contention.
    pub fn steal(&self) -> Option<T> {
        let inner = unsafe { &*self.inner };
        let t = inner.top.load(Ordering::Acquire);
        core::sync::atomic::fence(Ordering::SeqCst);
        let b = inner.bottom.load(Ordering::Acquire);

        if t < b {
            let buf = unsafe { &*inner.buffer.get() };
            let val = unsafe { buf.get(t) };

            match inner.top.compare_exchange(
                t,
                t + 1,
                Ordering::SeqCst,
                Ordering::Relaxed,
            ) {
                Ok(_) => Some(val),
                Err(_) => None, // contention, abort
            }
        } else {
            None
        }
    }

    /// Steal up to `count` elements from the top. Returns how many were stolen.
    pub fn steal_batch(&self, out: &mut [T]) -> usize {
        let inner = unsafe { &*self.inner };
        let t = inner.top.load(Ordering::Acquire);
        core::sync::atomic::fence(Ordering::SeqCst);
        let b = inner.bottom.load(Ordering::Acquire);

        let available = (b - t).max(0) as usize;
        if available == 0 {
            return 0;
        }

        let to_steal = available.min(out.len()).min(MAX_STEAL_BATCH);
        let buf = unsafe { &*inner.buffer.get() };

        // Read the elements.
        for i in 0..to_steal {
            out[i] = unsafe { buf.get(t + i as isize) };
        }

        // Advance the top pointer via CAS.
        match inner.top.compare_exchange(
            t,
            t + to_steal as isize,
            Ordering::SeqCst,
            Ordering::Relaxed,
        ) {
            Ok(_) => to_steal,
            Err(_) => 0, // contention, abort batch
        }
    }

    /// Approximate number of elements available to steal.
    pub fn len(&self) -> usize {
        let inner = unsafe { &*self.inner };
        let t = inner.top.load(Ordering::Relaxed);
        let b = inner.bottom.load(Ordering::Relaxed);
        (b - t).max(0) as usize
    }
}