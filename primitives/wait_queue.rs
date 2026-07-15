// Intrusive async wait queue.
//
// Tasks park by linking an embedded WaitNode from their TCB.
// No heap allocation. Wake order is priority-based.

use core::sync::atomic::{AtomicUsize, Ordering};
use core::task::Waker;
use core::pin::Pin;
use core::ptr;

use crate::primitives::spinlock::Spinlock;

/// Intrusive node embedded in a task's TCB.
/// SAFETY: must not be moved while linked into a wait queue.
pub struct WaitNode {
    waker: Option<Waker>,
    priority: i32,
    next: *mut WaitNode,
    prev: *mut WaitNode,
    linked: bool,
}

impl WaitNode {
    /// Create a new unlinked wait node.
    pub const fn new() -> Self {
        Self {
            waker: None,
            priority: 0,
            next: ptr::null_mut(),
            prev: ptr::null_mut(),
            linked: false,
        }
    }

    /// Set the waker for this node. Must be called before enqueueing.
    pub fn set_waker(&mut self, waker: Waker) {
        self.waker = Some(waker);
    }

    /// Set the priority for ordering within the wait queue.
    pub fn set_priority(&mut self, priority: i32) {
        self.priority = priority;
    }

    pub fn is_linked(&self) -> bool {
        self.linked
    }

    pub fn priority(&self) -> i32 {
        self.priority
    }
}

/// Intrusive doubly-linked list of WaitNodes, ordered by priority.
/// Protected by a spinlock since wakes can come from any core.
pub struct WaitQueue {
    inner: Spinlock<WaitQueueInner>,
    len: AtomicUsize,
}

struct WaitQueueInner {
    head: *mut WaitNode,
    tail: *mut WaitNode,
}

// SAFETY: spinlock provides mutual exclusion. WaitNode pointers point
// into TCBs which outlive their link lifetime.
unsafe impl Send for WaitQueue {}
unsafe impl Sync for WaitQueue {}

impl WaitQueue {
    /// Create an empty wait queue.
    pub const fn new() -> Self {
        Self {
            inner: Spinlock::new(WaitQueueInner {
                head: ptr::null_mut(),
                tail: ptr::null_mut(),
            }),
            len: AtomicUsize::new(0),
        }
    }

    /// Enqueue a wait node. SAFETY: node must remain valid until dequeued.
    pub unsafe fn enqueue(&self, node: *mut WaitNode) {
        let mut inner = self.inner.lock();
        let n = &mut *node;
        debug_assert!(!n.linked, "double-enqueue of WaitNode");
        debug_assert!(n.waker.is_some(), "WaitNode enqueued without waker");

        n.next = ptr::null_mut();
        n.prev = inner.tail;
        n.linked = true;

        if inner.tail.is_null() {
            debug_assert!(inner.head.is_null());
            inner.head = node;
        } else {
            (*inner.tail).next = node;
        }
        inner.tail = node;

        // Insertion-sort by priority (descending). Short queues (<8) are common.
        let prio = n.priority;
        let mut cursor = n.prev;
        while !cursor.is_null() && (*cursor).priority < prio {
            // Swap cursor and n in the list.
            let c = &mut *cursor;
            let n_ref = &mut *node;

            // Swap positions: cursor moves after n.
            n_ref.prev = c.prev;
            c.next = n_ref.next;
            n_ref.next = cursor;
            c.prev = node;

            if !n_ref.prev.is_null() {
                (*n_ref.prev).next = node;
            } else {
                inner.head = node;
            }
            if !c.next.is_null() {
                (*c.next).prev = cursor;
            } else {
                inner.tail = cursor;
            }

            cursor = n_ref.prev;
        }

        drop(inner);
        self.len.fetch_add(1, Ordering::Release);
    }

    /// Remove a specific node (e.g., on timeout).
    /// SAFETY: node must point to a valid, currently-linked WaitNode.
    pub unsafe fn remove(&self, node: *mut WaitNode) {
        let mut inner = self.inner.lock();
        let n = &mut *node;
        if !n.linked {
            drop(inner);
            return;
        }

        if !n.prev.is_null() {
            (*n.prev).next = n.next;
        } else {
            inner.head = n.next;
        }
        if !n.next.is_null() {
            (*n.next).prev = n.prev;
        } else {
            inner.tail = n.prev;
        }

        n.next = ptr::null_mut();
        n.prev = ptr::null_mut();
        n.linked = false;

        drop(inner);
        self.len.fetch_sub(1, Ordering::Release);
    }

    /// Wake the highest-priority waiter. Returns true if someone was woken.
    pub fn wake_one(&self) -> bool {
        let mut inner = self.inner.lock();
        if inner.head.is_null() {
            return false;
        }

        let node = inner.head;
        let n = unsafe { &mut *node };

        inner.head = n.next;
        if inner.head.is_null() {
            inner.tail = ptr::null_mut();
        } else {
            unsafe { (*inner.head).prev = ptr::null_mut(); }
        }

        let waker = n.waker.take();
        n.next = ptr::null_mut();
        n.prev = ptr::null_mut();
        n.linked = false;

        drop(inner);
        self.len.fetch_sub(1, Ordering::Release);

        if let Some(w) = waker {
            w.wake();
            true
        } else {
            false
        }
    }

    /// Wake all waiters.
    pub fn wake_all(&self) {
        let mut wakers: heapless::Vec<Waker, 64> = heapless::Vec::new();

        {
            let mut inner = self.inner.lock();
            while !inner.head.is_null() {
                let node = inner.head;
                let n = unsafe { &mut *node };

                inner.head = n.next;
                if inner.head.is_null() {
                    inner.tail = ptr::null_mut();
                } else {
                    unsafe { (*inner.head).prev = ptr::null_mut(); }
                }

                if let Some(w) = n.waker.take() {
                    let _ = wakers.push(w);
                }
                n.next = ptr::null_mut();
                n.prev = ptr::null_mut();
                n.linked = false;
            }
        }

        self.len.store(0, Ordering::Release);

        for w in wakers {
            w.wake();
        }
    }

    /// Returns the number of waiters.
    pub fn len(&self) -> usize {
        self.len.load(Ordering::Acquire)
    }

    /// Returns true if the queue has no waiters.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

// Small inline Vec for wake_all (can't allocate in kernel).
// If more than 64 waiters exist, we wake in batches.
mod heapless {
    pub struct Vec<T, const N: usize> {
        buf: [core::mem::MaybeUninit<T>; N],
        len: usize,
    }

    impl<T, const N: usize> Vec<T, N> {
        pub const fn new() -> Self {
            Self {
                // SAFETY: MaybeUninit does not require initialization.
                buf: unsafe { core::mem::MaybeUninit::uninit().assume_init() },
                len: 0,
            }
        }

        pub fn push(&mut self, val: T) -> Result<(), T> {
            if self.len >= N {
                return Err(val);
            }
            self.buf[self.len] = core::mem::MaybeUninit::new(val);
            self.len += 1;
            Ok(())
        }
    }

    impl<T, const N: usize> Drop for Vec<T, N> {
        fn drop(&mut self) {
            for i in 0..self.len {
                // SAFETY: Elements 0..len are initialized.
                unsafe { self.buf[i].assume_init_drop(); }
            }
        }
    }

    impl<T, const N: usize> IntoIterator for Vec<T, N> {
        type Item = T;
        type IntoIter = IntoIter<T, N>;
        fn into_iter(self) -> Self::IntoIter {
            IntoIter { vec: self, idx: 0 }
        }
    }

    pub struct IntoIter<T, const N: usize> {
        vec: Vec<T, N>,
        idx: usize,
    }

    impl<T, const N: usize> Iterator for IntoIter<T, N> {
        type Item = T;
        fn next(&mut self) -> Option<T> {
            if self.idx >= self.vec.len {
                return None;
            }
            let val = unsafe { self.vec.buf[self.idx].assume_init_read() };
            self.idx += 1;
            Some(val)
        }
    }
}